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

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SIGMACODE_API_KEY` | — | Your LLM API key (required) |
| `SIGMACODE_MODEL` | `gpt-4o` | Model to use |
| `SIGMACODE_BASE_URL` | `https://api.openai.com/v1` | API base URL |
| `SIGMACODE_PROVIDER` | `openai` | Provider: `openai`, `anthropic`, `gemini`, `ollama` |

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
│   ├── sigmacode-cli/        # Headless CLI
│   ├── sigmacode-retrieval/  # Context retrieval (WIP)
│   └── sigmacode-sandbox/    # OS sandboxing (WIP)
└── .env                      # Your config (git-ignored)
```

## License

MIT
