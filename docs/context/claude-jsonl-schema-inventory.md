# Claude JSONL Schema Inventory

Generated at: 1771631205 (unix epoch seconds, UTC) / 2026-02-20T23:46:45Z (ISO 8601)

## Scan Scope

- Mode: `latest`
- Transcript glob: `/Users/will/.claude/projects/*/*.jsonl`
- Time filter: `last 24 hour(s)`
- Files scanned: 8
- Total JSONL records: 45243
- JSON parse errors skipped: 0
- Latest transcript (sorted by mtime): `/Users/will/.claude/projects/-Users-will-Documents-Areas-TemplateRepos-malcomsonbrothers-b2b-rust-axum-sveltekit-monorepo/1bc17c58-743a-4480-8730-669354ae8173.jsonl`
- Latest transcript mtime: 1771630650 (unix epoch seconds) / 2026-02-20T23:37:30Z (ISO 8601)
- Canonical field paths discovered: 441
- Top-level record types discovered: 9

## Schema/Version Probe

- Fields containing `schema`: none found.
- Fields containing `version` (top 10 by prevalence):
  - `version`
  - `snapshot.trackedFileBackups.{path}.version`
  - `message.content[].input.version`

The selected file list is saved to:

- `docs/context/claude-jsonl-selected-files.txt`

## Top Record Types

| Type | Count | Percent of records |
|---|---:|---:|
| `progress` | 24019 | 53.09% |
| `assistant` | 9948 | 21.99% |
| `user` | 6698 | 14.80% |
| `system` | 1855 | 4.10% |
| `file-history-snapshot` | 1647 | 3.64% |
| `queue-operation` | 1049 | 2.32% |
| `pr-link` | 20 | 0.04% |
| `summary` | 4 | 0.01% |
| `custom-title` | 3 | 0.01% |

## Top Field Paths

| Field path | Count | Percent of records |
|---|---:|---:|
| `type` | 45243 | 100.00% |
| `sessionId` | 43592 | 96.35% |
| `timestamp` | 43589 | 96.34% |
| `cwd` | 42520 | 93.98% |
| `gitBranch` | 42520 | 93.98% |
| `isSidechain` | 42520 | 93.98% |
| `parentUuid` | 42520 | 93.98% |
| `userType` | 42520 | 93.98% |
| `uuid` | 42520 | 93.98% |
| `version` | 42520 | 93.98% |
| `slug` | 42491 | 93.92% |
| `toolUseID` | 25403 | 56.15% |
| `data` | 24019 | 53.09% |
| `data.type` | 24019 | 53.09% |
| `parentToolUseID` | 24019 | 53.09% |
| `message` | 16646 | 36.79% |
| `message.content` | 16646 | 36.79% |
| `message.role` | 16646 | 36.79% |
| `message.content[].type` | 14977 | 33.10% |
| `data.command` | 14478 | 32.00% |
| `data.hookEvent` | 14478 | 32.00% |
| `data.hookName` | 14478 | 32.00% |
| `message.id` | 9948 | 21.99% |
| `message.model` | 9948 | 21.99% |
| `message.stop_reason` | 9948 | 21.99% |
| `message.stop_sequence` | 9948 | 21.99% |
| `message.type` | 9948 | 21.99% |
| `message.usage` | 9948 | 21.99% |
| `message.usage.cache_creation` | 9948 | 21.99% |
| `message.usage.cache_creation.ephemeral_1h_input_tokens` | 9948 | 21.99% |

## Full Outputs

- Field-level stats with descriptions:
  - `docs/context/claude-jsonl-field-stats.csv`
- Record type stats:
  - `docs/context/claude-jsonl-type-stats.csv`

## Re-run Commands

```bash
# Rebuild from latest 20 transcript files (newest first)
cargo xtask schema-inventory --latest 20

# Rebuild from latest 20 files modified in the last 24 hours
cargo xtask schema-inventory --latest 20 --since-hours 24

# Rebuild from all files modified in the last 7 days
cargo xtask schema-inventory --all --since-days 7

# Rebuild from all transcript files
cargo xtask schema-inventory --all
```
