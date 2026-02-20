use chrono::{SecondsFormat, TimeZone, Utc};
use clap::{Args, Parser, Subcommand};
use glob::glob;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type DynError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Parser, Debug)]
#[command(name = "xtask")]
#[command(about = "Project automation tasks.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate a schema inventory from Claude transcript JSONL files.
    SchemaInventory(SchemaInventoryArgs),
}

#[derive(Args, Debug)]
struct SchemaInventoryArgs {
    /// Scan all transcript files (otherwise scans latest N files).
    #[arg(long)]
    all: bool,

    /// Scan the N most recent transcript files (ignored when --all is set).
    #[arg(long, default_value_t = 20)]
    latest: usize,

    /// Only include transcript files modified in the last H hours.
    #[arg(long, conflicts_with = "since_days")]
    since_hours: Option<u64>,

    /// Only include transcript files modified in the last D days.
    #[arg(long, conflicts_with = "since_hours")]
    since_days: Option<u64>,

    /// Output directory for generated inventory artifacts.
    #[arg(long, default_value = "docs/context")]
    out_dir: PathBuf,

    /// Glob for transcript JSONL files.
    #[arg(long, default_value = "~/.claude/projects/*/*.jsonl")]
    glob: String,
}

#[derive(Debug, Clone)]
struct FileMeta {
    path: PathBuf,
    mtime_epoch: u64,
}

#[derive(Debug, Clone)]
enum PathSeg {
    Key(String),
    Array,
}

fn main() -> Result<(), DynError> {
    let cli = Cli::parse();
    match cli.command {
        Command::SchemaInventory(args) => run_schema_inventory(args),
    }
}

fn run_schema_inventory(args: SchemaInventoryArgs) -> Result<(), DynError> {
    if !args.all && args.latest == 0 {
        return Err("`--latest` must be greater than 0.".into());
    }

    fs::create_dir_all(&args.out_dir)?;

    let transcript_glob = expand_tilde(&args.glob)?;
    let mut all_files = collect_files_sorted_by_mtime(&transcript_glob)?;
    if all_files.is_empty() {
        return Err(format!("No transcript files found for glob: {transcript_glob}").into());
    }

    let (time_filter_desc, cutoff_epoch) = build_time_filter(args.since_hours, args.since_days)?;
    if let Some(cutoff) = cutoff_epoch {
        all_files.retain(|f| f.mtime_epoch >= cutoff);
    }

    if all_files.is_empty() {
        return Err(format!(
            "No transcript files found for glob/time filter: {} ({})",
            transcript_glob, time_filter_desc
        )
        .into());
    }

    let selected_files: Vec<FileMeta> = if args.all {
        all_files
    } else {
        all_files.into_iter().take(args.latest).collect()
    };

    if selected_files.is_empty() {
        return Err("No transcript files selected after filtering.".into());
    }

    let selected_list_path = args.out_dir.join("claude-jsonl-selected-files.txt");
    write_selected_files_list(&selected_list_path, &selected_files)?;

    let mut total_records: u64 = 0;
    let mut type_counts: HashMap<String, u64> = HashMap::new();
    let mut field_counts: HashMap<String, u64> = HashMap::new();
    let mut parse_errors: u64 = 0;

    for file in &selected_files {
        let f = File::open(&file.path)?;
        let reader = BufReader::new(f);

        for line_result in reader.lines() {
            let line = line_result?;
            if line.trim().is_empty() {
                continue;
            }

            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    parse_errors += 1;
                    continue;
                }
            };

            total_records += 1;

            let record_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("<missing>")
                .to_string();
            increment_count(&mut type_counts, record_type);

            let mut field_set: BTreeSet<String> = BTreeSet::new();
            let mut segs: Vec<PathSeg> = Vec::new();
            collect_field_paths(&value, &mut segs, &mut field_set);
            for field in field_set {
                increment_count(&mut field_counts, field);
            }
        }
    }

    if total_records == 0 {
        return Err("Selected transcript files contain zero parseable JSONL records.".into());
    }

    let mut sorted_types: Vec<(String, u64)> = type_counts.into_iter().collect();
    sorted_types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut sorted_fields: Vec<(String, u64)> = field_counts.into_iter().collect();
    sorted_fields.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let type_csv_path = args.out_dir.join("claude-jsonl-type-stats.csv");
    let field_csv_path = args.out_dir.join("claude-jsonl-field-stats.csv");
    let report_path = args.out_dir.join("claude-jsonl-schema-inventory.md");

    write_type_csv(&type_csv_path, total_records, &sorted_types)?;
    write_field_csv(&field_csv_path, total_records, &sorted_fields)?;
    write_markdown_report(
        &report_path,
        &args.out_dir,
        &transcript_glob,
        args.all,
        &time_filter_desc,
        &selected_files,
        total_records,
        parse_errors,
        &sorted_types,
        &sorted_fields,
    )?;

    println!("Wrote:");
    println!("  - {}", report_path.display());
    println!("  - {}", field_csv_path.display());
    println!("  - {}", type_csv_path.display());
    println!("  - {}", selected_list_path.display());

    Ok(())
}

fn build_time_filter(
    since_hours: Option<u64>,
    since_days: Option<u64>,
) -> Result<(String, Option<u64>), DynError> {
    let now_epoch = epoch_now()?;
    match (since_hours, since_days) {
        (Some(h), None) => {
            if h == 0 {
                return Err("`--since-hours` must be greater than 0.".into());
            }
            let cutoff = now_epoch.saturating_sub(h.saturating_mul(3600));
            Ok((format!("last {} hour(s)", h), Some(cutoff)))
        }
        (None, Some(d)) => {
            if d == 0 {
                return Err("`--since-days` must be greater than 0.".into());
            }
            let cutoff = now_epoch.saturating_sub(d.saturating_mul(86_400));
            Ok((format!("last {} day(s)", d), Some(cutoff)))
        }
        (None, None) => Ok(("none".to_string(), None)),
        (Some(_), Some(_)) => Err("Use either `--since-hours` or `--since-days`, not both.".into()),
    }
}

fn collect_files_sorted_by_mtime(glob_pattern: &str) -> Result<Vec<FileMeta>, DynError> {
    let mut files: Vec<FileMeta> = Vec::new();
    for entry in glob(glob_pattern)? {
        let path = match entry {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !path.is_file() {
            continue;
        }
        let metadata = fs::metadata(&path)?;
        let modified = metadata.modified()?;
        let mtime_epoch = to_epoch(modified)?;
        files.push(FileMeta { path, mtime_epoch });
    }

    files.sort_by(|a, b| {
        b.mtime_epoch
            .cmp(&a.mtime_epoch)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(files)
}

fn collect_field_paths(value: &Value, segs: &mut Vec<PathSeg>, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                segs.push(PathSeg::Key(key.clone()));
                out.insert(canonical_path(segs));
                collect_field_paths(child, segs, out);
                segs.pop();
            }
        }
        Value::Array(items) => {
            for item in items {
                segs.push(PathSeg::Array);
                collect_field_paths(item, segs, out);
                segs.pop();
            }
        }
        _ => {}
    }
}

fn canonical_path(segs: &[PathSeg]) -> String {
    let mut result = String::new();
    let mut after_tracked_file_backups = false;

    for seg in segs {
        match seg {
            PathSeg::Array => {
                result.push_str("[]");
            }
            PathSeg::Key(key) => {
                let part = if after_tracked_file_backups {
                    after_tracked_file_backups = false;
                    "{path}"
                } else {
                    key.as_str()
                };

                if !result.is_empty() {
                    result.push('.');
                }
                result.push_str(part);

                if part == "trackedFileBackups" {
                    after_tracked_file_backups = true;
                }
            }
        }
    }

    result
}

fn write_selected_files_list(path: &Path, files: &[FileMeta]) -> Result<(), DynError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for f in files {
        writeln!(writer, "{}", f.path.display())?;
    }
    writer.flush()?;
    Ok(())
}

fn write_type_csv(
    path: &Path,
    total_records: u64,
    items: &[(String, u64)],
) -> Result<(), DynError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "\"type\",\"count\",\"percent_of_records\"")?;
    for (typ, count) in items {
        let pct = (*count as f64 / total_records as f64) * 100.0;
        writeln!(
            writer,
            "{},{},{}",
            csv_escape(typ),
            count,
            format!("{pct:.6}")
        )?;
    }
    writer.flush()?;
    Ok(())
}

fn write_field_csv(
    path: &Path,
    total_records: u64,
    items: &[(String, u64)],
) -> Result<(), DynError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    writeln!(
        writer,
        "\"field_path\",\"count\",\"percent_of_records\",\"description\""
    )?;
    for (field, count) in items {
        let pct = (*count as f64 / total_records as f64) * 100.0;
        let desc = describe_field(field);
        writeln!(
            writer,
            "{},{},{},{}",
            csv_escape(field),
            count,
            format!("{pct:.6}"),
            csv_escape(&desc)
        )?;
    }
    writer.flush()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_markdown_report(
    report_path: &Path,
    out_dir: &Path,
    transcript_glob: &str,
    all_mode: bool,
    time_filter_desc: &str,
    selected_files: &[FileMeta],
    total_records: u64,
    parse_errors: u64,
    sorted_types: &[(String, u64)],
    sorted_fields: &[(String, u64)],
) -> Result<(), DynError> {
    let mode = if all_mode { "all" } else { "latest" };
    let generated_at_epoch = epoch_now()?;
    let generated_at_iso = epoch_to_iso8601_utc(generated_at_epoch);
    let latest_file = &selected_files[0];
    let latest_mtime_iso = epoch_to_iso8601_utc(latest_file.mtime_epoch);

    let top_types = markdown_rows(sorted_types.iter().take(12), total_records);
    let top_fields = markdown_rows(sorted_fields.iter().take(30), total_records);
    let schema_like_fields: Vec<&str> = sorted_fields
        .iter()
        .map(|(field, _)| field.as_str())
        .filter(|field| field.to_ascii_lowercase().contains("schema"))
        .collect();
    let version_like_fields: Vec<&str> = sorted_fields
        .iter()
        .map(|(field, _)| field.as_str())
        .filter(|field| field.to_ascii_lowercase().contains("version"))
        .take(10)
        .collect();

    let mut report = String::new();
    report.push_str("# Claude JSONL Schema Inventory\n\n");
    report.push_str(&format!(
        "Generated at: {} (unix epoch seconds, UTC) / {} (ISO 8601)\n\n",
        generated_at_epoch, generated_at_iso
    ));
    report.push_str("## Scan Scope\n\n");
    report.push_str(&format!("- Mode: `{mode}`\n"));
    report.push_str(&format!("- Transcript glob: `{transcript_glob}`\n"));
    report.push_str(&format!("- Time filter: `{time_filter_desc}`\n"));
    report.push_str(&format!("- Files scanned: {}\n", selected_files.len()));
    report.push_str(&format!("- Total JSONL records: {total_records}\n"));
    report.push_str(&format!("- JSON parse errors skipped: {parse_errors}\n"));
    report.push_str(&format!(
        "- Latest transcript (sorted by mtime): `{}`\n",
        latest_file.path.display()
    ));
    report.push_str(&format!(
        "- Latest transcript mtime: {} (unix epoch seconds) / {} (ISO 8601)\n",
        latest_file.mtime_epoch, latest_mtime_iso
    ));
    report.push_str(&format!(
        "- Canonical field paths discovered: {}\n",
        sorted_fields.len()
    ));
    report.push_str(&format!(
        "- Top-level record types discovered: {}\n\n",
        sorted_types.len()
    ));

    report.push_str("## Schema/Version Probe\n\n");
    if schema_like_fields.is_empty() {
        report.push_str("- Fields containing `schema`: none found.\n");
    } else {
        report.push_str("- Fields containing `schema`:\n");
        for field in &schema_like_fields {
            report.push_str(&format!("  - `{field}`\n"));
        }
    }
    if version_like_fields.is_empty() {
        report.push_str("- Fields containing `version`: none found.\n\n");
    } else {
        report.push_str("- Fields containing `version` (top 10 by prevalence):\n");
        for field in &version_like_fields {
            report.push_str(&format!("  - `{field}`\n"));
        }
        report.push('\n');
    }

    report.push_str("The selected file list is saved to:\n\n");
    report.push_str(&format!(
        "- `{}`\n\n",
        out_dir.join("claude-jsonl-selected-files.txt").display()
    ));

    report.push_str("## Top Record Types\n\n");
    report.push_str("| Type | Count | Percent of records |\n");
    report.push_str("|---|---:|---:|\n");
    report.push_str(&top_types);
    report.push('\n');

    report.push_str("## Top Field Paths\n\n");
    report.push_str("| Field path | Count | Percent of records |\n");
    report.push_str("|---|---:|---:|\n");
    report.push_str(&top_fields);
    report.push('\n');

    report.push_str("## Full Outputs\n\n");
    report.push_str("- Field-level stats with descriptions:\n");
    report.push_str(&format!(
        "  - `{}`\n",
        out_dir.join("claude-jsonl-field-stats.csv").display()
    ));
    report.push_str("- Record type stats:\n");
    report.push_str(&format!(
        "  - `{}`\n\n",
        out_dir.join("claude-jsonl-type-stats.csv").display()
    ));

    report.push_str("## Re-run Commands\n\n");
    report.push_str("```bash\n");
    report.push_str("# Rebuild from latest 20 transcript files (newest first)\n");
    report.push_str("cargo xtask schema-inventory --latest 20\n\n");
    report.push_str("# Rebuild from latest 20 files modified in the last 24 hours\n");
    report.push_str("cargo xtask schema-inventory --latest 20 --since-hours 24\n\n");
    report.push_str("# Rebuild from all files modified in the last 7 days\n");
    report.push_str("cargo xtask schema-inventory --all --since-days 7\n\n");
    report.push_str("# Rebuild from all transcript files\n");
    report.push_str("cargo xtask schema-inventory --all\n");
    report.push_str("```\n");

    fs::write(report_path, report)?;
    Ok(())
}

fn markdown_rows<'a>(rows: impl Iterator<Item = &'a (String, u64)>, total_records: u64) -> String {
    let mut out = String::new();
    for (name, count) in rows {
        let pct = (*count as f64 / total_records as f64) * 100.0;
        out.push_str(&format!("| `{}` | {} | {:.2}% |\n", name, count, pct));
    }
    out
}

fn describe_field(path: &str) -> String {
    if path == "type" {
        return "Top-level record category.".to_string();
    }
    if path == "timestamp" {
        return "Event timestamp in ISO-8601 format.".to_string();
    }
    if path == "uuid" {
        return "Unique identifier for this log record.".to_string();
    }
    if path == "sessionId" {
        return "Conversation/session identifier.".to_string();
    }
    if path == "parentUuid" {
        return "Parent log record identifier.".to_string();
    }
    if path == "version" {
        return "Claude/Codex runtime version string.".to_string();
    }
    if path == "cwd" {
        return "Working directory at event time.".to_string();
    }
    if path == "gitBranch" {
        return "Active git branch at event time.".to_string();
    }
    if path == "slug" {
        return "Human-readable session slug.".to_string();
    }
    if path == "message" {
        return "Top-level message payload.".to_string();
    }
    if path == "message.role" {
        return "Speaker role inside message payload.".to_string();
    }
    if path == "message.content" {
        return "Message content container.".to_string();
    }
    if path == "message.content[].type" {
        return "Typed content block category (text, thinking, tool_use, tool_result, image, document).".to_string();
    }
    if path == "message.content[].text" {
        return "Plain text content from a message block.".to_string();
    }
    if path == "message.content[].thinking" {
        return "Model reasoning/thinking text block.".to_string();
    }
    if path == "message.content[].signature" {
        return "Signature metadata for thinking blocks.".to_string();
    }
    if path == "message.content[].tool_use_id" {
        return "Tool invocation identifier referenced by a tool result.".to_string();
    }
    if path == "message.content[].name" {
        return "Tool name for tool_use blocks.".to_string();
    }
    if path == "message.content[].id" {
        return "Tool-use content block identifier.".to_string();
    }
    if path.starts_with("message.content[].input") {
        return "Tool invocation input payload (shape varies by tool).".to_string();
    }
    if path.starts_with("message.content[].caller") {
        return "Origin metadata for a tool invocation.".to_string();
    }
    if path == "message.content[].content" {
        return "Tool result content payload.".to_string();
    }
    if path == "message.content[].is_error" {
        return "Flag indicating tool result represents an error.".to_string();
    }
    if path == "message.model" {
        return "Model identifier used for this assistant response.".to_string();
    }
    if path == "message.id" {
        return "Model message identifier.".to_string();
    }
    if path == "message.stop_reason" {
        return "Why model output stopped.".to_string();
    }
    if path == "message.stop_sequence" {
        return "Matched stop sequence, when present.".to_string();
    }
    if path.starts_with("message.usage") {
        return "Token/service usage accounting for this assistant message.".to_string();
    }
    if path == "requestId" {
        return "Backend request identifier.".to_string();
    }
    if path == "toolUseID" {
        return "Primary tool-use identifier for progress/system records.".to_string();
    }
    if path == "parentToolUseID" {
        return "Parent tool-use identifier.".to_string();
    }
    if path == "sourceToolAssistantUUID" {
        return "Assistant UUID that originated a tool use/result.".to_string();
    }
    if path.starts_with("toolUseResult") {
        return "Serialized local tool execution result payload.".to_string();
    }
    if path == "data" {
        return "Progress payload container.".to_string();
    }
    if path == "data.type" {
        return "Progress subtype (hook_progress, bash_progress, agent_progress, etc).".to_string();
    }
    if path == "data.command" {
        return "Command associated with hook/progress events.".to_string();
    }
    if path == "data.hookEvent" {
        return "Lifecycle stage for hook progress.".to_string();
    }
    if path == "data.hookName" {
        return "Hook identifier.".to_string();
    }
    if path == "data.totalLines" {
        return "Line count reported by streamed command output.".to_string();
    }
    if path == "data.output" {
        return "Incremental command output content.".to_string();
    }
    if path == "data.fullOutput" {
        return "Full command output snapshot.".to_string();
    }
    if path == "data.elapsedTimeSeconds" {
        return "Execution duration in seconds.".to_string();
    }
    if path == "data.timeoutMs" {
        return "Timeout in milliseconds for operation.".to_string();
    }
    if path == "data.prompt" {
        return "Prompt text for background/agent task.".to_string();
    }
    if path == "data.agentId" {
        return "Background agent identifier.".to_string();
    }
    if path.starts_with("data.normalizedMessages") {
        return "Normalized message payload emitted by agent progress.".to_string();
    }
    if path.starts_with("data.message") {
        return "Embedded message record within progress events.".to_string();
    }
    if path == "subtype" {
        return "Subtype field for system events.".to_string();
    }
    if path == "permissionMode" {
        return "Permission mode in effect for user action.".to_string();
    }
    if path == "level" {
        return "Severity/suggestion level for system events.".to_string();
    }
    if path == "stopReason" {
        return "Stop reason summary for stop-hook system messages.".to_string();
    }
    if path == "preventedContinuation" {
        return "Whether a hook prevented continuation.".to_string();
    }
    if path == "hasOutput" {
        return "Whether a hook produced output.".to_string();
    }
    if path == "hookCount" {
        return "Count of hooks involved in a summary event.".to_string();
    }
    if path.starts_with("hookInfos") {
        return "Hook execution metadata list.".to_string();
    }
    if path == "hookErrors" {
        return "Hook error list.".to_string();
    }
    if path == "todos" {
        return "Todo list captured on user records.".to_string();
    }
    if path.starts_with("thinkingMetadata") {
        return "Reasoning mode metadata for a user request.".to_string();
    }
    if path == "snapshot" {
        return "File-history snapshot payload.".to_string();
    }
    if path == "messageId" {
        return "Message identifier referenced by snapshot/system records.".to_string();
    }
    if path == "isSnapshotUpdate" {
        return "Whether file-history snapshot is incremental.".to_string();
    }
    if path == "snapshot.timestamp" {
        return "Timestamp of file-history snapshot.".to_string();
    }
    if path == "snapshot.messageId" {
        return "Message identifier that produced this snapshot.".to_string();
    }
    if path == "snapshot.trackedFileBackups" {
        return "Map of file backup entries captured at snapshot time.".to_string();
    }
    if path.starts_with("snapshot.trackedFileBackups.{path}") {
        return "Tracked backup metadata for a specific file path (path key is canonicalized)."
            .to_string();
    }
    if path == "operation" {
        return "Queue operation type (enqueue, dequeue, remove, popAll).".to_string();
    }
    if path == "isMeta" {
        return "Flag indicating metadata-only user record.".to_string();
    }
    if path == "summary" {
        return "Conversation summary text.".to_string();
    }
    if path == "leafUuid" {
        return "Leaf UUID associated with summary records.".to_string();
    }
    if path == "customTitle" {
        return "User-defined session title.".to_string();
    }
    if path == "prUrl" {
        return "Pull request URL referenced by session metadata.".to_string();
    }
    if path == "prNumber" {
        return "Pull request number referenced by session metadata.".to_string();
    }
    if path == "prRepository" {
        return "Repository slug for PR metadata.".to_string();
    }

    let last = path.rsplit('.').next().unwrap_or(path).replace("[]", "");
    format!("Auto-generated: field {last} in path {path}.")
}

fn csv_escape(input: &str) -> String {
    format!("\"{}\"", input.replace('\"', "\"\""))
}

fn increment_count(map: &mut HashMap<String, u64>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

fn epoch_now() -> Result<u64, DynError> {
    to_epoch(SystemTime::now())
}

fn to_epoch(t: SystemTime) -> Result<u64, DynError> {
    Ok(t.duration_since(UNIX_EPOCH)?.as_secs())
}

fn epoch_to_iso8601_utc(epoch: u64) -> String {
    match Utc.timestamp_opt(epoch as i64, 0).single() {
        Some(dt) => dt.to_rfc3339_opts(SecondsFormat::Secs, true),
        None => "invalid-epoch".to_string(),
    }
}

fn expand_tilde(pattern: &str) -> Result<String, DynError> {
    if let Some(stripped) = pattern.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME is not set; cannot expand '~' in glob pattern.")?;
        return Ok(format!("{home}/{stripped}"));
    }
    if pattern == "~" {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME is not set; cannot expand '~' in glob pattern.")?;
        return Ok(home);
    }
    Ok(pattern.to_string())
}
