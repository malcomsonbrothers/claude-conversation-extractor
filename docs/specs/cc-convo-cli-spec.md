# `cc-convo` CLI Specification (Rust)

## 1. Objective

Build a Rust-native CLI named `cc-convo` that extracts, searches, views, and exports Claude local conversation transcripts from `~/.claude/projects`.

This replaces the Python reference implementation with:

- stronger schema tolerance for evolving JSONL formats
- clear subcommand-based UX
- deterministic output contracts
- testable parser and formatter layers

## 2. Rust CLI Package Stack

Primary CLI stack (from `nogit-reference/rust-cli-packages.md`):

- `clap`: required; argument parsing, subcommands, help, validation
- `dialoguer`: used for interactive prompts (optional flows like confirm/export selections)
- `console`: used for styled terminal output and consistent color handling
- `indicatif`: used for progress bars/spinners during long scans/exports

Additional implementation crates:

- `serde` + `serde_json`: JSONL decoding and tolerant parsing
- `walkdir`/`glob`: transcript discovery
- `regex`: search mode
- `chrono`: timestamp parsing/formatting

## 3. Inputs and Schema Baseline

Baseline source:

- `docs/context/claude-jsonl-schema-inventory.md`
- generated from latest transcripts with `--since-hours 24`

Observed top-level record types (24h sample):

- `progress`
- `assistant`
- `user`
- `system`
- `file-history-snapshot`
- `queue-operation`
- `pr-link`
- `summary`
- `custom-title`

Important compatibility note:

- no explicit `schema` field observed
- must parse defensively and ignore unknown fields by default

## 4. Command Surface

Binary:

- `cc-convo`

Primary subcommands:

1. `cc-convo sessions list`
2. `cc-convo sessions show <session-id|index>`
3. `cc-convo export`
4. `cc-convo search <query>`
5. `cc-convo stats`
6. `cc-convo doctor`

Optional compatibility aliases (for migration ergonomics):

- `cc-convo list` -> `sessions list`
- `cc-convo view` -> `sessions show`

## 5. Global Options

Supported on all subcommands:

- `--claude-dir <path>` (default `~/.claude/projects`)
- `--json` (machine-readable CLI output)
- `--verbose`
- `--no-color`

Time filtering (applies to session/file mtime):

- `--since-hours <n>`
- `--since-days <n>`
- `--until <iso8601>`

## 6. Sessions Commands

### 6.1 `sessions list`

Purpose:

- list available sessions sorted by transcript mtime descending

Options:

- `--limit <n>` (default 50)
- `--project <name|path-substring>`
- `--with-preview` (first meaningful user prompt)

Output columns (table mode):

- index
- session id (full + short)
- project
- modified time (ISO 8601)
- file size
- message counts (`user`/`assistant`/other)

### 6.2 `sessions show`

Purpose:

- print normalized transcript for one session in terminal

Options:

- `--detailed` (include non-text blocks and operational events)
- `--max-lines <n>`
- `--raw` (show raw JSON line objects)

## 7. Export Command

### 7.1 `export`

Purpose:

- export one or multiple sessions to files

Target selection (mutually combinable where sensible):

- `--session <id>` (repeatable)
- `--index <n>` (repeatable)
- `--recent <n>`
- `--all`
- `--search "<query>"` (export search-matched sessions)

Output options:

- `--format <markdown|json|html>` (default `markdown`)
- `--output <dir>` (default `./cc-convo-exports`)
- `--detailed`
- `--single-file` (concatenate)

Filename contract:

- `cc-convo-<YYYY-MM-DD>-<session-short>.<ext>`

## 8. Search Command

### 8.1 `search`

Purpose:

- full-text search across normalized transcript content

Options:

- `--mode <smart|exact|regex>` (default `smart`)
- `--speaker <user|assistant|both>` (default `both`)
- `--case-sensitive`
- `--max-results <n>` (default 30)
- `--context-chars <n>` (default 150)

Result fields:

- session id
- project
- timestamp (if available)
- speaker
- relevance
- preview snippet

## 9. Stats Command

### 9.1 `stats`

Purpose:

- summarize corpus-level metadata

Outputs:

- session count
- record type distribution
- block type distribution (`text`, `tool_use`, `tool_result`, `thinking`, etc.)
- model usage distribution (from assistant records)
- parser skip/error counts

## 10. Doctor Command

### 10.1 `doctor`

Purpose:

- validate environment and transcript readability

Checks:

- transcript root exists and is readable
- at least one JSONL file found
- parse sample from latest files
- permission and output-dir writeability

## 11. Normalization Rules

### 11.1 Default extraction mode

Include:

- `user` message textual content
- `assistant` `message.content[]` blocks where `type == "text"`

Exclude:

- progress/system/queue records
- binary payload blocks (`image`, `document`) unless `--detailed`
- meta-only command caveat text unless `--raw`

### 11.2 Detailed mode

Include additionally:

- assistant `thinking` blocks
- `tool_use` blocks (`id`, `name`, `input`)
- `tool_result` blocks (`tool_use_id`, `content`, `is_error`)
- selected system/progress summaries (not raw spam by default)

### 11.3 Timestamp behavior

- preserve source `timestamp` when present
- fallback to transcript mtime for ordering metadata
- render all user-facing timestamps in ISO 8601

## 12. Data Model (Rust)

Core structs:

- `TranscriptFile { path, mtime, session_id, project }`
- `RawRecord { type, timestamp, session_id, ... }` (serde with unknown fields)
- `NormalizedEvent { role, content, timestamp, source_type, metadata }`
- `ExportDocument { session_meta, events, stats }`

Serde guidance:

- use permissive `Option<T>` fields
- retain unknown keys in `serde_json::Value` map for future compatibility

## 13. Implementation Plan

Suggested internal modules:

- `cmd/` (`sessions`, `export`, `search`, `stats`, `doctor`)
- `transcript/discovery.rs`
- `transcript/parser.rs`
- `transcript/normalize.rs`
- `format/markdown.rs`, `format/json.rs`, `format/html.rs`
- `search/engine.rs`

## 14. Acceptance Criteria

1. `cc-convo sessions list` shows latest sessions in descending mtime order.
2. `cc-convo export --recent 5 --format markdown` writes 5 files with deterministic names.
3. `cc-convo search "tool_use"` returns matches with session context.
4. Unknown record types/fields do not crash parsing.
5. All timestamps displayed in ISO 8601.
6. Tests cover parser behavior for current observed record and block types.
