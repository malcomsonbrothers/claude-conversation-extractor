# claude-conversation-extractor

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
