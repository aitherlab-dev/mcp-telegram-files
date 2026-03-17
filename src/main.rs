use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process;

const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB Telegram limit

// --- MCP Protocol Types ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

// --- Telegram Config ---

#[derive(Deserialize)]
struct TelegramConfig {
    chat_id: Value,
}

// --- Core Logic ---

fn get_bot_token() -> Result<String, String> {
    let entry = keyring::Entry::new("aitherflow", "telegram-bot-token")
        .map_err(|e| {
            eprintln!("[mcp-telegram-files] Failed to access keyring: {e}");
            format!("Failed to access keyring: {e}")
        })?;
    entry.get_password().map_err(|e| {
        eprintln!("[mcp-telegram-files] Failed to get bot token from keyring: {e}");
        format!("Failed to get bot token from keyring: {e}")
    })
}

fn get_telegram_config() -> Result<TelegramConfig, String> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Cannot determine config directory".to_string())?;
    let config_path = config_dir.join("aither-flow").join("telegram.json");
    let content = fs::read_to_string(&config_path).map_err(|e| {
        eprintln!(
            "[mcp-telegram-files] Failed to read {}: {e}",
            config_path.display()
        );
        format!("Failed to read {}: {e}", config_path.display())
    })?;
    serde_json::from_str(&content).map_err(|e| {
        eprintln!("[mcp-telegram-files] Failed to parse telegram.json: {e}");
        format!("Failed to parse telegram.json: {e}")
    })
}

fn send_file_to_telegram(
    path: &str,
    client: &reqwest::blocking::Client,
) -> Result<String, String> {
    let file_path = PathBuf::from(path);
    let file_path = if file_path.is_relative() {
        std::env::current_dir()
            .map_err(|e| format!("Cannot get current dir: {e}"))?
            .join(&file_path)
    } else {
        file_path
    };

    let file_path = fs::canonicalize(&file_path)
        .map_err(|e| format!("Cannot resolve path '{}': {e}", file_path.display()))?;

    let metadata = fs::metadata(&file_path)
        .map_err(|e| format!("Cannot access file '{}': {e}", file_path.display()))?;

    if !metadata.is_file() {
        return Err(format!("'{}' is not a regular file", file_path.display()));
    }

    if metadata.len() > MAX_FILE_SIZE {
        return Err(format!(
            "File too large: {} MB (Telegram limit is 50 MB)",
            metadata.len() / (1024 * 1024)
        ));
    }

    let bot_token = get_bot_token()?;
    let config = get_telegram_config()?;

    let chat_id = match &config.chat_id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err("Invalid chat_id in telegram.json".to_string()),
    };

    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let file_bytes =
        fs::read(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;

    let part = reqwest::blocking::multipart::Part::bytes(file_bytes)
        .file_name(file_name.clone())
        .mime_str("application/octet-stream")
        .map_err(|e| format!("Failed to create multipart part: {e}"))?;

    let form = reqwest::blocking::multipart::Form::new()
        .text("chat_id", chat_id)
        .part("document", part);

    let url = format!("https://api.telegram.org/bot{bot_token}/sendDocument");

    let resp = client.post(&url).multipart(form).send().map_err(|e| {
        // Filter token from error message to prevent leaking
        let msg = e.to_string().replace(&bot_token, "<TOKEN>");
        eprintln!("[mcp-telegram-files] HTTP request failed: {msg}");
        format!("HTTP request to Telegram failed: {msg}")
    })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .map_err(|e| format!("Failed to read Telegram response: {e}"))?;
    let body: Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Telegram response: {e}"))?;

    if status.is_success() && body.get("ok") == Some(&json!(true)) {
        Ok(format!("File sent: {file_name}"))
    } else {
        let description = body
            .get("description")
            .and_then(|d: &Value| d.as_str())
            .unwrap_or("Unknown error");
        eprintln!("[mcp-telegram-files] Telegram API error: {description}");
        Err(format!("Telegram API error: {description}"))
    }
}

// --- MCP Handlers ---

fn handle_initialize(id: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "mcp-telegram-files",
                "version": "0.1.0"
            }
        })),
        error: None,
    }
}

fn handle_tools_list(id: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(json!({
            "tools": [
                {
                    "name": "send_file_to_telegram",
                    "description": "Send a file to the configured Telegram chat via Bot API",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Absolute or relative path to the file to send"
                            }
                        },
                        "required": ["path"]
                    }
                }
            ]
        })),
        error: None,
    }
}

fn handle_tools_call(
    id: Value,
    params: &Value,
    client: &reqwest::blocking::Client,
) -> JsonRpcResponse {
    let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");

    if tool_name != "send_file_to_telegram" {
        return JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(json!({
                "code": -32602,
                "message": format!("Unknown tool: {tool_name}")
            })),
        };
    }

    let path = params
        .get("arguments")
        .and_then(|a| a.get("path"))
        .and_then(|p| p.as_str())
        .unwrap_or("");

    if path.is_empty() {
        return JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "content": [{"type": "text", "text": "Error: 'path' parameter is required"}],
                "isError": true
            })),
            error: None,
        };
    }

    match send_file_to_telegram(path, client) {
        Ok(msg) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "content": [{"type": "text", "text": msg}]
            })),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "content": [{"type": "text", "text": format!("Error: {e}")}],
                "isError": true
            })),
            error: None,
        },
    }
}

// --- Helpers ---

fn write_response(stdout: &mut io::Stdout, data: &str) {
    if writeln!(stdout, "{data}").is_err() || stdout.flush().is_err() {
        eprintln!("[mcp-telegram-files] Stdout pipe broken, exiting");
        process::exit(0);
    }
}

// --- Main Loop ---

fn main() {
    eprintln!("[mcp-telegram-files] Server starting");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let client = reqwest::blocking::Client::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[mcp-telegram-files] Stdin read error: {e}");
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let error_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {e}")}
                });
                write_response(&mut stdout, &error_resp.to_string());
                continue;
            }
        };

        // Notifications (no id) don't need a response
        if request.id.is_none() {
            continue;
        }

        let id = request.id.unwrap();

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(id),
            "tools/list" => handle_tools_list(id),
            "tools/call" => handle_tools_call(id, &request.params, &client),
            _ => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: None,
                error: Some(json!({
                    "code": -32601,
                    "message": format!("Method not found: {}", request.method)
                })),
            },
        };

        let json_str = serde_json::to_string(&response).expect("Failed to serialize response");
        write_response(&mut stdout, &json_str);
    }

    eprintln!("[mcp-telegram-files] Server shutting down");
}
