# claude-conversation-extractor

## `cc-convo` CLI (Rust)

The main CLI binary is `cc-convo`.

```bash
# Run directly from workspace
cargo run -p cc-convo -- --help

# List latest sessions (sorted by transcript mtime desc)
cargo run -p cc-convo -- sessions list --limit 10

# Show one session by index or id
cargo run -p cc-convo -- sessions show 1

# Search corpus
cargo run -p cc-convo -- search "tool_use" --max-results 20

# Export last 5 sessions as markdown
cargo run -p cc-convo -- export --recent 5 --format markdown

# Health checks
cargo run -p cc-convo -- doctor

# Generate shell completions
cargo run -p cc-convo -- completions zsh > _cc-convo
cargo run -p cc-convo -- completions bash > cc-convo.bash
cargo run -p cc-convo -- completions fish > cc-convo.fish
```

Global time filters are supported across commands:

- `--since-hours <n>`
- `--since-days <n>`
- `--until <RFC3339>`

## Xtask Automation

This repo uses a Rust `xtask` command for transcript schema inventory generation.

```bash
# Latest 20 transcript files
cargo xtask schema-inventory --latest 20

# Only files modified in the last 24 hours
cargo xtask schema-inventory --latest 20 --since-hours 24

# All files modified in the last 7 days
cargo xtask schema-inventory --all --since-days 7
```

Generated outputs are written to `docs/context/`:

- `claude-jsonl-schema-inventory.md`
- `claude-jsonl-field-stats.csv`
- `claude-jsonl-type-stats.csv`
- `claude-jsonl-selected-files.txt`
