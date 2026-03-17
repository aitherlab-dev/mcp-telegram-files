# mcp-telegram-files

MCP server for sending files to Telegram via Claude Code.

## What it does

A stdio-based [Model Context Protocol](https://modelcontextprotocol.io/) server that provides the `send_file_to_telegram` tool. Claude agents can send any file (up to 50 MB) as a Telegram document to your configured chat.

## Prerequisites

- Rust toolchain (1.70+)
- A Telegram bot created via [BotFather](https://t.me/BotFather)
- [aitherflow](https://github.com/aitherlab-dev/aitherflow) configuration:
  - `~/.config/aither-flow/telegram.json` with your `chat_id`
  - Bot token stored in the system keyring (service: `aitherflow`, key: `telegram-bot-token`)

## Build

```bash
cargo build --release
```

## Install

Register the MCP server in Claude Code:

```bash
claude mcp add --scope user telegram-files /path/to/target/release/mcp-telegram-files
```

## Usage

Ask any Claude agent:

> "Send me the file src/main.rs"

The file arrives in your Telegram chat as a document.

## Configuration

### telegram.json

Create `~/.config/aither-flow/telegram.json`:

```json
{
  "chat_id": 123456789,
  "enabled": true
}
```

Replace `123456789` with your Telegram chat ID (you can get it from [@userinfobot](https://t.me/userinfobot)).

### Keyring setup

Store your bot token in the system keyring:

```bash
secret-tool store --label="telegram-bot-token" service aitherflow username telegram-bot-token
```

Then enter your bot token when prompted.

## License

[MIT](LICENSE)
