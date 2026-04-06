# rose-offline

This is a modified version of the rose offline server and files to support new functionality.

## Features

- **MCP and REST API**: Control in-game bots via LLM through a comprehensive API
- **LLM Feedback Loop**: Autonomous bot control with intelligent decision-making (see [LLM Feedback System](docs/llm-feedback-system.md))

### LLM Feedback System

The server includes an optional LLM feedback system that enables bots to:
- Follow players and respond to chat messages
- Participate in combat with intelligent decision-making
- Make context-aware decisions based on game state
- Respond to player commands naturally

**Enable the feature:**
```bash
cargo build --features llm-feedback
```

**Configuration (environment variables):**
- `LLM_SERVER_URL` - LLM server endpoint (default: `http://localhost:8080`)
- `LLM_API_KEY` - API key for authentication (default: `any-key-works`)
- `LLM_ENABLED` - Enable/disable LLM feedback (default: `true`)

See [docs/llm-feedback-system.md](docs/llm-feedback-system.md) for complete documentation.

---

ORIGINAL SOURCE HERE:
https://github.com/exjam/rose-offline/

---

An open source server for ROSE Online, compatible with the official 129_129en irose client or [rose-offline-client](https://github.com/exjam/rose-offline-client).

# Running the server
Run rose-offline-server from your installed official client directory (the folder containing data.idx), or you can use the `--data-idx` or `--data-path` arguments as described below.

## Optional arguments:
- `--data-idx=<path/to/data.idx>` Path to irose 129en data.idx
- `--data-path=<path/to/data>` Path to extracted irose 129en game files
- `--ip=<ip>` IP to listen for client connections, defaults to 127.0.0.1
