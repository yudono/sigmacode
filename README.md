# SigmaCode

A modern agentic AI coding assistant built in Rust. Inspired by Claude Code, OpenCode, Gemini CLI, and Codex.

SigmaCode uses an LLM as a reasoning engine with retrieval, tools, planning, verification, and feedback loops — the same architecture behind today's best coding agents.

## Features

- **ReAct Agent Loop** — Reason, Act, Observe, Repeat
- **Multi-Provider LLM** — OpenAI, Anthropic, Gemini, Ollama
- **6 Built-in Tools** — read, write, edit, bash, glob, grep
- **Streaming TUI** — Real-time terminal UI with vim-like keybindings
- **Auto-Compaction** — Manages context window automatically
- **Planner** — LLM-based task decomposition
- **Working Memory** — Budget-aware memory system

## Quick Start

### 1. Install

```bash
git clone https://github.com/yudono/sigmacode.git
cd sigmacode
cargo build --release
```

The binary will be at `target/release/sigmacode`.

### 2. Configure

Create a `.env` file in your project root:

```bash
# Required: Your API key
SIGMACODE_API_KEY=your-api-key-here

# Optional: Model (default: gpt-4o)
SIGMACODE_MODEL=gpt-4o

# Optional: Base URL for OpenAI-compatible APIs
SIGMACODE_BASE_URL=https://api.openai.com/v1

# Optional: Provider type (openai, anthropic, gemini, ollama)
SIGMACODE_PROVIDER=openai
```

Or set environment variables directly:

```bash
export SIGMACODE_API_KEY="your-key"
export SIGMACODE_MODEL="gpt-4o"
```

### 3. Run

**TUI mode** (interactive terminal):
```bash
sigmacode
```

**Headless mode** (automation/scripting):
```bash
sigmacode-headless "Add Google OAuth to the app"
```

**Server mode** (HTTP API):
```bash
# Start the server on default port 3847
sigmacode-server

# Or configure via environment
SIGMACODE_PORT=3847 \
SIGMACODE_API_KEY=your-key \
SIGMACODE_BASE_URL=https://api.openai.com/v1 \
SIGMACODE_MODEL=gpt-4o \
sigmacode-server
```

The server exposes:
- `GET /health` — health check
- `POST /api/chat` — non-streaming chat
- `POST /api/chat/stream` — SSE streaming chat
- `GET /api/sessions` — list sessions
- `POST /api/sessions` — create session
- `GET /api/sessions/:id` — get session
- `DELETE /api/sessions/:id` — delete session
- `POST /api/sessions/:id/messages` — add message to session
- `GET /config` — provider info

### Server Security

Protect API routes with a server key:

```bash
# Store key in Redis (encrypted with AES-256-GCM)
cargo run --bin sigmacode-keys -- store "my-secret-key"

# Or use env var directly
SERVER_KEY=my-secret-key sigmacode-server
```

When `SERVER_KEY` is set, all `/api/*` requests must include:
```
Authorization: Bearer <server-key>
```

### Server with Redis (encrypted key storage)

```bash
REDIS_URL=redis://127.0.0.1:6379 \
ENCRYPTION_KEY=02163eb0a4d05fd3265610aa4b6df4812be1d9a8510b7badea13bb625df5496d \
SERVER_KEY=my-secret-key \
sigmacode-server
```

The key is encrypted before storing in Redis and decrypted at startup.

### API Examples

**Chat (non-streaming):**
```bash
curl -X POST http://localhost:3847/api/chat \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{"message": "Create a React todo app", "mode": "builder"}'
```

**Stream (SSE):**
```bash
curl -N -X POST http://localhost:3847/api/chat/stream \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{"message": "List files in src/", "mode": "chat"}'
```

**Create session:**
```bash
curl -X POST http://localhost:3847/api/sessions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-secret-key" \
  -d '{"title": "Build API", "mode": "builder"}'
```

### Execution Modes

| Mode | Description |
|------|-------------|
| `chat` | Direct LLM, no tools — fast for questions and simple tasks |
| `planner` | Analyze + plan only — shows decomposition without executing |
| `builder` | Full orchestrator pipeline — plan, execute, verify, review |

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SIGMACODE_API_KEY` | — | Your LLM API key (required) |
| `SIGMACODE_MODEL` | `gpt-4o` | Model to use |
| `SIGMACODE_BASE_URL` | `https://api.openai.com/v1` | API base URL |
| `SIGMACODE_PROVIDER` | `openai` | Provider: `openai`, `anthropic`, `gemini`, `ollama` |
| `SIGMACODE_PORT` | `3847` | Server port (server mode only) |
| `SERVER_KEY` | — | Auth key for API routes (server mode only) |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Redis for encrypted key storage |
| `ENCRYPTION_KEY` | — | Master key for AES-256-GCM key encryption |

### Provider Examples

**OpenAI:**
```bash
SIGMACODE_API_KEY=sk-...
SIGMACODE_MODEL=gpt-4o
SIGMACODE_PROVIDER=openai
```

**Anthropic:**
```bash
SIGMACODE_API_KEY=sk-ant-...
SIGMACODE_MODEL=claude-sonnet-4-20250514
SIGMACODE_PROVIDER=anthropic
```

**Ollama (local):**
```bash
SIGMACODE_MODEL=codellama
SIGMACODE_PROVIDER=ollama
SIGMACODE_BASE_URL=http://localhost:11434
```

**Custom OpenAI-compatible:**
```bash
SIGMACODE_API_KEY=your-key
SIGMACODE_MODEL=your-model
SIGMACODE_BASE_URL=https://your-api.com/v1
SIGMACODE_PROVIDER=openai
```

## TUI Controls

| Key | Action |
|-----|--------|
| `i` | Enter input mode (type your task) |
| `Enter` | Submit task |
| `Esc` | Cancel input |
| `l` | Switch to Logs view |
| `c` | Switch to Chat view |
| `q` | Quit (when idle) |
| `Ctrl+C` | Force quit |

## Built-in Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents with line numbers |
| `write_file` | Create or overwrite files |
| `edit_file` | Find-and-replace in files (exact match) |
| `bash` | Execute shell commands |
| `glob` | Find files by pattern |
| `grep` | Search file contents with regex |

## How It Works

```
User Input
    │
    ▼
┌─────────────────┐
│   Planner       │  Decompose task into steps
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  LLM Reasoning  │  Decide which tool to use
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Tool Execution │  Read/write/edit files, run commands
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Verification   │  Check if task is complete
└────────┬────────┘
         │
         ▼
     Done / Loop
```

## Project Structure

```
sigmacode/
├── crates/
│   ├── sigmacode-core/       # Core agent engine
│   ├── sigmacode-tui/        # Terminal UI
│   ├── sigmacode-cli/        # Headless CLI, HTTP server, key manager
│   ├── sigmacode-retrieval/  # Context retrieval (WIP)
│   └── sigmacode-sandbox/    # OS sandboxing (WIP)
├── target/release/
│   ├── sigmacode             # TUI binary
│   ├── sigmacode-headless    # Headless CLI
│   ├── sigmacode-server      # HTTP API server
│   └── sigmacode-keys        # Key encryption tool
└── .env                      # Your config (git-ignored)
```

## License

MIT
