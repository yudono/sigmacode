# SigmaCode Project Instructions

## Build & Verify

After every code change, run:
```bash
cargo test
```
This is mandatory before committing. 55+ tests must all pass.

## Architecture

- Rust workspace: `crates/sigmacode-core`, `crates/sigmacode-tui`, `crates/sigmacode-cli`
- LLM provider: MiMo API (`api.xiaomimimo.com/v1`, model `mimo-v2.5`)
- Tools via system prompt text (MiMo doesn't support native function calling)
- Config at `~/.sigma/config.yml` (YAML)

## Key Conventions

- MiMo messages: assistant tool_calls embedded as text in content, tool role converted to user role
- Streaming tokens may contain `<tool_result>` XML tags — must be stripped in TUI
- `parse_tool_calls_from_text` uses brace-depth scanner for finding JSON anywhere in text
- `ToolCallStarted` event shows args_summary — TUI parses JSON for display
- Graph-flow crate logs to stderr via `log` crate — suppressed via NullLogger in TUI main

## Commit Style

```
type: short description

Detailed explanation if needed.
```

Types: `feat`, `fix`, `refactor`, `build`, `test`
