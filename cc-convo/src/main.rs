use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use console::style;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use regex::RegexBuilder;
use serde::Serialize;
use serde_json::{json, Value};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "cc-convo")]
#[command(about = "Extract, search, and export Claude local conversations.")]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
struct GlobalArgs {
    #[arg(long, default_value = "~/.claude/projects")]
    claude_dir: String,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    verbose: bool,
    #[arg(long)]
    no_color: bool,
    #[arg(long, conflicts_with = "since_days")]
    since_hours: Option<u64>,
    #[arg(long, conflicts_with = "since_hours")]
    since_days: Option<u64>,
    #[arg(long, help = "Upper bound mtime filter in ISO 8601 / RFC3339 format.")]
    until: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    Export(ExportArgs),
    Search(SearchArgs),
    Stats(StatsArgs),
    Doctor(DoctorArgs),
    #[command(hide = true)]
    List(SessionsListArgs),
    #[command(hide = true)]
    View(SessionsShowArgs),
}

#[derive(Subcommand, Debug)]
enum SessionsCommand {
    List(SessionsListArgs),
    Show(SessionsShowArgs),
}

#[derive(Args, Debug)]
struct SessionsListArgs {
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[arg(long, help = "Filter by project name/path substring.")]
    project: Option<String>,
    #[arg(long)]
    with_preview: bool,
}

#[derive(Args, Debug)]
struct SessionsShowArgs {
    target: String,
    #[arg(long)]
    detailed: bool,
    #[arg(long)]
    max_lines: Option<usize>,
    #[arg(long)]
    raw: bool,
}

#[derive(Args, Debug)]
struct ExportArgs {
    #[arg(long = "session", action = clap::ArgAction::Append)]
    sessions: Vec<String>,
    #[arg(long = "index", action = clap::ArgAction::Append)]
    indices: Vec<usize>,
    #[arg(long)]
    recent: Option<usize>,
    #[arg(long)]
    all: bool,
    #[arg(long, help = "Select sessions that match this query before exporting.")]
    search: Option<String>,
    #[arg(long, value_enum, default_value_t = ExportFormat::Markdown)]
    format: ExportFormat,
    #[arg(long, default_value = "cc-convo-exports")]
    output: PathBuf,
    #[arg(long)]
    detailed: bool,
    #[arg(long)]
    single_file: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Copy, Clone, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    Markdown,
    Json,
    Html,
}

#[derive(Args, Debug)]
struct SearchArgs {
    query: String,
    #[arg(long, value_enum, default_value_t = SearchMode::Smart)]
    mode: SearchMode,
    #[arg(long, value_enum, default_value_t = SpeakerFilter::Both)]
    speaker: SpeakerFilter,
    #[arg(long)]
    case_sensitive: bool,
    #[arg(long, default_value_t = 30)]
    max_results: usize,
    #[arg(long, default_value_t = 150)]
    context_chars: usize,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum SearchMode {
    Smart,
    Exact,
    Regex,
}

#[derive(Copy, Clone, Debug, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SpeakerFilter {
    User,
    Assistant,
    Both,
}

#[derive(Args, Debug)]
struct StatsArgs {
    #[arg(long, default_value_t = 20)]
    top: usize,
}

#[derive(Args, Debug)]
struct DoctorArgs {
    #[arg(long, default_value_t = 5)]
    sample_files: usize,
    #[arg(long, default_value = "cc-convo-exports")]
    output: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct Session {
    index: usize,
    id: String,
    id_short: String,
    project: String,
    path: PathBuf,
    modified_iso: String,
    modified_epoch: i64,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct SessionSummary {
    session: Session,
    user_messages: u64,
    assistant_messages: u64,
    other_records: u64,
    preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedEvent {
    role: String,
    source_type: String,
    timestamp: Option<String>,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct SearchHit {
    session_id: String,
    project: String,
    path: PathBuf,
    speaker: String,
    timestamp: Option<String>,
    relevance: f64,
    preview: String,
}

#[derive(Debug, Clone, Serialize)]
struct ParseOutput {
    events: Vec<NormalizedEvent>,
    parse_errors: u64,
}

#[derive(Debug, Clone)]
struct TimeWindow {
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.global.no_color {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }

    let time_window = time_window_from_global(&cli.global)?;
    let claude_dir = expand_tilde_path(&cli.global.claude_dir)?;

    match cli.command {
        Command::Sessions { command } => match command {
            SessionsCommand::List(args) => {
                cmd_sessions_list(&claude_dir, &time_window, &cli.global, args)
            }
            SessionsCommand::Show(args) => {
                cmd_sessions_show(&claude_dir, &time_window, &cli.global, args)
            }
        },
        Command::List(args) => cmd_sessions_list(&claude_dir, &time_window, &cli.global, args),
        Command::View(args) => cmd_sessions_show(&claude_dir, &time_window, &cli.global, args),
        Command::Export(args) => cmd_export(&claude_dir, &time_window, &cli.global, args),
        Command::Search(args) => cmd_search(&claude_dir, &time_window, &cli.global, args),
        Command::Stats(args) => cmd_stats(&claude_dir, &time_window, &cli.global, args),
        Command::Doctor(args) => cmd_doctor(&claude_dir, &time_window, &cli.global, args),
    }
}

fn cmd_sessions_list(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: SessionsListArgs,
) -> Result<()> {
    let mut sessions = discover_sessions(claude_dir, time_window)?;
    if let Some(project_filter) = args.project {
        let project_filter = project_filter.to_lowercase();
        sessions.retain(|s| {
            s.project.to_lowercase().contains(&project_filter)
                || s.path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&project_filter)
        });
    }

    let sessions = sessions.into_iter().take(args.limit).collect::<Vec<_>>();
    let mut summaries = Vec::with_capacity(sessions.len());
    for session in sessions {
        let summary = summarize_session(&session, args.with_preview)?;
        summaries.push(summary);
    }

    if global.json {
        print_json(&summaries)?;
        return Ok(());
    }

    print_sessions_table(&summaries, args.with_preview);
    Ok(())
}

fn cmd_sessions_show(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: SessionsShowArgs,
) -> Result<()> {
    let sessions = discover_sessions(claude_dir, time_window)?;
    let session = resolve_session_target(&sessions, &args.target)?;

    if args.raw {
        let records = read_raw_lines(&session.path)?;
        if global.json {
            print_json(&json!({
                "session": session,
                "records": records,
            }))?;
        } else {
            println!("{}", style(format!("Session {}", session.id)).bold().cyan());
            for record in records {
                println!("{record}");
            }
        }
        return Ok(());
    }

    let parsed = parse_session_events(&session.path, args.detailed)?;
    let events = if let Some(max) = args.max_lines {
        parsed.events.into_iter().take(max).collect::<Vec<_>>()
    } else {
        parsed.events
    };

    if global.json {
        print_json(&json!({
            "session": session,
            "parse_errors": parsed.parse_errors,
            "events": events,
        }))?;
        return Ok(());
    }

    println!("{}", style(format!("Session {}", session.id)).bold().cyan());
    println!("Project: {}", session.project);
    println!("Modified: {}", session.modified_iso);
    println!("Path: {}", session.path.display());
    println!();
    for event in events {
        let ts = event.timestamp.unwrap_or_else(|| "-".to_string());
        println!(
            "{} {} {}",
            style(ts).dim(),
            style(format!("[{}]", event.role)).bold(),
            event.content
        );
    }
    if parsed.parse_errors > 0 {
        eprintln!(
            "{}",
            style(format!(
                "Skipped {} malformed JSON lines.",
                parsed.parse_errors
            ))
            .yellow()
        );
    }
    Ok(())
}

fn cmd_export(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: ExportArgs,
) -> Result<()> {
    let sessions = discover_sessions(claude_dir, time_window)?;
    let selected = select_sessions_for_export(&sessions, &args)?;

    if selected.is_empty() {
        bail!("No sessions selected for export.");
    }

    if args.all && !args.yes && !global.json {
        let proceed = Confirm::new()
            .with_prompt(format!("Export all {} sessions?", selected.len()))
            .default(false)
            .interact()
            .context("Failed to read confirmation input")?;
        if !proceed {
            println!("Cancelled.");
            return Ok(());
        }
    }

    fs::create_dir_all(&args.output)
        .with_context(|| format!("Failed to create output dir {}", args.output.display()))?;

    let pb = if !global.json && selected.len() > 1 {
        let pb = ProgressBar::new(selected.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
        );
        Some(pb)
    } else {
        None
    };

    let mut output_files = Vec::new();
    let mut bundled_docs = Vec::new();
    let mut total_parse_errors = 0u64;
    let mut exported = 0usize;

    for session in &selected {
        let parsed = parse_session_events(&session.path, args.detailed)?;
        total_parse_errors += parsed.parse_errors;
        let doc = build_export_document(session, &parsed.events);
        if args.single_file {
            bundled_docs.push(doc);
        } else {
            let path = write_single_export(&args.output, &doc, args.format)?;
            output_files.push(path);
        }
        exported += 1;
        if let Some(pb) = &pb {
            pb.set_message(session.id_short.clone());
            pb.inc(1);
        }
    }

    if let Some(pb) = &pb {
        pb.finish_with_message("done");
    }

    if args.single_file {
        let path = write_bundle_export(&args.output, &bundled_docs, args.format)?;
        output_files.push(path);
    }

    if global.json {
        print_json(&json!({
            "exported_sessions": exported,
            "output_files": output_files,
            "parse_errors": total_parse_errors,
            "format": args.format,
            "detailed": args.detailed,
            "single_file": args.single_file
        }))?;
        return Ok(());
    }

    println!(
        "{}",
        style(format!("Exported {} session(s).", exported))
            .bold()
            .green()
    );
    println!("Output:");
    for p in &output_files {
        println!("  {}", p.display());
    }
    if total_parse_errors > 0 {
        eprintln!(
            "{}",
            style(format!(
                "Skipped {} malformed JSON lines.",
                total_parse_errors
            ))
            .yellow()
        );
    }
    Ok(())
}

fn cmd_search(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: SearchArgs,
) -> Result<()> {
    let sessions = discover_sessions(claude_dir, time_window)?;
    let hits = search_sessions(&sessions, &args)?;

    let hits = hits.into_iter().take(args.max_results).collect::<Vec<_>>();
    if global.json {
        print_json(&hits)?;
        return Ok(());
    }

    println!(
        "{}",
        style(format!("Found {} result(s).", hits.len()))
            .bold()
            .cyan()
    );
    for (i, hit) in hits.iter().enumerate() {
        println!();
        println!(
            "{} {} {}",
            style(format!("#{}", i + 1)).bold(),
            style(&hit.session_id).green(),
            style(format!("({})", hit.project)).dim()
        );
        println!(
            "{} {} {:.2}",
            style(hit.timestamp.clone().unwrap_or_else(|| "-".into())).dim(),
            style(format!("[{}]", hit.speaker)).bold(),
            hit.relevance
        );
        println!("{}", hit.preview);
    }
    Ok(())
}

fn cmd_stats(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: StatsArgs,
) -> Result<()> {
    let sessions = discover_sessions(claude_dir, time_window)?;
    let mut record_type_counts: HashMap<String, u64> = HashMap::new();
    let mut block_type_counts: HashMap<String, u64> = HashMap::new();
    let mut model_counts: HashMap<String, u64> = HashMap::new();
    let mut parse_errors: u64 = 0;
    let mut total_records: u64 = 0;

    for session in &sessions {
        let f = File::open(&session.path)?;
        let reader = BufReader::new(f);
        for line in reader.lines() {
            let line = line?;
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
            *record_type_counts.entry(record_type).or_insert(0) += 1;

            if value.get("type").and_then(Value::as_str) == Some("assistant") {
                if let Some(model) = value
                    .get("message")
                    .and_then(|m| m.get("model"))
                    .and_then(Value::as_str)
                {
                    *model_counts.entry(model.to_string()).or_insert(0) += 1;
                }
            }

            if let Some(content) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
            {
                for item in content {
                    if let Some(t) = item.get("type").and_then(Value::as_str) {
                        *block_type_counts.entry(t.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let record_type_top = top_n_sorted_map(record_type_counts, args.top);
    let block_type_top = top_n_sorted_map(block_type_counts, args.top);
    let model_top = top_n_sorted_map(model_counts, args.top);

    if global.json {
        print_json(&json!({
            "sessions": sessions.len(),
            "total_records": total_records,
            "parse_errors": parse_errors,
            "record_types": record_type_top,
            "content_block_types": block_type_top,
            "models": model_top
        }))?;
        return Ok(());
    }

    println!("{}", style("Corpus stats").bold().cyan());
    println!("Sessions: {}", sessions.len());
    println!("Records: {}", total_records);
    println!("Parse errors: {}", parse_errors);
    println!();
    print_ranked_map("Top record types", &record_type_top);
    println!();
    print_ranked_map("Top content block types", &block_type_top);
    println!();
    print_ranked_map("Top models", &model_top);
    Ok(())
}

fn cmd_doctor(
    claude_dir: &Path,
    time_window: &TimeWindow,
    global: &GlobalArgs,
    args: DoctorArgs,
) -> Result<()> {
    let mut checks = Vec::new();
    checks.push(check_path_exists("claude_dir_exists", claude_dir));
    checks.push(check_path_readable("claude_dir_readable", claude_dir));

    let sessions = discover_sessions(claude_dir, time_window).unwrap_or_default();
    checks.push(CheckResult::new(
        "jsonl_files_found",
        !sessions.is_empty(),
        format!("found {}", sessions.len()),
    ));

    let sample = sessions.iter().take(args.sample_files).collect::<Vec<_>>();
    let mut sample_parse_errors = 0u64;
    let mut sample_records = 0u64;
    for session in sample {
        let f = File::open(&session.path)
            .with_context(|| format!("Failed to open {}", session.path.display()))?;
        let reader = BufReader::new(f);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            sample_records += 1;
            if serde_json::from_str::<Value>(&line).is_err() {
                sample_parse_errors += 1;
            }
        }
    }
    checks.push(CheckResult::new(
        "sample_parse",
        sample_records > 0 && sample_parse_errors == 0,
        format!(
            "records={} parse_errors={}",
            sample_records, sample_parse_errors
        ),
    ));

    let writable = ensure_output_dir_writable(&args.output).is_ok();
    checks.push(CheckResult::new(
        "output_dir_writable",
        writable,
        args.output.display().to_string(),
    ));

    if global.json {
        print_json(&checks)?;
        return Ok(());
    }

    println!("{}", style("Doctor").bold().cyan());
    for c in &checks {
        let status = if c.ok {
            style("OK").green()
        } else {
            style("FAIL").red()
        };
        println!("{status} {:<24} {}", c.name, c.details);
    }

    let failed = checks.iter().filter(|c| !c.ok).count();
    if failed > 0 {
        bail!("Doctor found {failed} failing checks.");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct CheckResult {
    name: String,
    ok: bool,
    details: String,
}

impl CheckResult {
    fn new(name: impl Into<String>, ok: bool, details: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok,
            details: details.into(),
        }
    }
}

fn check_path_exists(name: &str, path: &Path) -> CheckResult {
    CheckResult::new(name, path.exists(), path.display().to_string())
}

fn check_path_readable(name: &str, path: &Path) -> CheckResult {
    let ok = fs::read_dir(path).is_ok();
    CheckResult::new(name, ok, path.display().to_string())
}

fn ensure_output_dir_writable(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let test = path.join(".cc-convo-write-test");
    fs::write(&test, b"ok")?;
    fs::remove_file(test)?;
    Ok(())
}

fn discover_sessions(claude_dir: &Path, time_window: &TimeWindow) -> Result<Vec<Session>> {
    if !claude_dir.exists() {
        bail!("Claude directory does not exist: {}", claude_dir.display());
    }
    let mut sessions = Vec::new();
    for entry in WalkDir::new(claude_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = fs::metadata(path)?;
        let modified = metadata
            .modified()
            .with_context(|| format!("Failed to get mtime for {}", path.display()))?;
        let modified_dt: DateTime<Utc> = modified.into();
        if let Some(since) = time_window.since {
            if modified_dt < since {
                continue;
            }
        }
        if let Some(until) = time_window.until {
            if modified_dt > until {
                continue;
            }
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid file stem for {}", path.display()))?
            .to_string();
        let project = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let modified_iso = modified_dt.to_rfc3339_opts(SecondsFormat::Secs, true);
        let modified_epoch = modified_dt.timestamp();
        let size_bytes = metadata.len();

        sessions.push(Session {
            index: 0,
            id_short: short_id(&stem),
            id: stem,
            project,
            path: path.to_path_buf(),
            modified_iso,
            modified_epoch,
            size_bytes,
        });
    }

    sessions.sort_by_key(|s| (Reverse(s.modified_epoch), s.path.clone()));
    for (i, session) in sessions.iter_mut().enumerate() {
        session.index = i + 1;
    }
    Ok(sessions)
}

fn summarize_session(session: &Session, with_preview: bool) -> Result<SessionSummary> {
    let mut user = 0u64;
    let mut assistant = 0u64;
    let mut other = 0u64;

    let mut preview = None;
    let f = File::open(&session.path)?;
    let reader = BufReader::new(f);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                user += 1;
                if with_preview && preview.is_none() {
                    let p = extract_message_text(&value, false);
                    if !p.trim().is_empty() {
                        preview = Some(clean_preview(&p));
                    }
                }
            }
            Some("assistant") => assistant += 1,
            _ => other += 1,
        }
    }

    Ok(SessionSummary {
        session: session.clone(),
        user_messages: user,
        assistant_messages: assistant,
        other_records: other,
        preview,
    })
}

fn parse_session_events(path: &Path, detailed: bool) -> Result<ParseOutput> {
    let f = File::open(path)?;
    let reader = BufReader::new(f);
    let mut events = Vec::new();
    let mut parse_errors = 0u64;

    for line in reader.lines() {
        let line = line?;
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
        let record_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        match record_type {
            "user" => {
                let text = extract_message_text(&value, detailed);
                if !text.trim().is_empty() {
                    events.push(NormalizedEvent {
                        role: "user".to_string(),
                        source_type: "user".to_string(),
                        timestamp,
                        content: text,
                    });
                }
            }
            "assistant" => {
                let text = extract_message_text(&value, detailed);
                if !text.trim().is_empty() {
                    events.push(NormalizedEvent {
                        role: "assistant".to_string(),
                        source_type: "assistant".to_string(),
                        timestamp,
                        content: text,
                    });
                }
            }
            "system" | "progress" | "queue-operation" | "file-history-snapshot" => {
                if detailed {
                    let short = summarize_non_dialog_record(&value);
                    events.push(NormalizedEvent {
                        role: record_type.to_string(),
                        source_type: record_type.to_string(),
                        timestamp,
                        content: short,
                    });
                }
            }
            _ => {
                if detailed {
                    events.push(NormalizedEvent {
                        role: record_type.to_string(),
                        source_type: record_type.to_string(),
                        timestamp,
                        content: truncate_value(&value, 500),
                    });
                }
            }
        }
    }

    Ok(ParseOutput {
        events,
        parse_errors,
    })
}

fn extract_message_text(record: &Value, detailed: bool) -> String {
    let Some(message) = record.get("message") else {
        return String::new();
    };
    let Some(content) = message.get("content") else {
        return String::new();
    };
    extract_content_text(content, detailed)
}

fn extract_content_text(content: &Value, detailed: bool) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                let item_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
                match item_type {
                    "text" => {
                        if let Some(txt) = obj.get("text").and_then(Value::as_str) {
                            parts.push(txt.to_string());
                        }
                    }
                    "thinking" if detailed => {
                        let thinking = obj.get("thinking").and_then(Value::as_str).unwrap_or("");
                        parts.push(format!("[thinking]\n{thinking}"));
                    }
                    "tool_use" if detailed => {
                        let name = obj.get("name").and_then(Value::as_str).unwrap_or("unknown");
                        let input = obj.get("input").cloned().unwrap_or_else(|| json!({}));
                        parts.push(format!(
                            "[tool_use] {}\n{}",
                            name,
                            serde_json::to_string_pretty(&input)
                                .unwrap_or_else(|_| "{}".to_string())
                        ));
                    }
                    "tool_result" if detailed => {
                        let tool_use_id = obj
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        let result_content = obj.get("content").cloned().unwrap_or(Value::Null);
                        parts.push(format!(
                            "[tool_result] {}\n{}",
                            tool_use_id,
                            truncate_value(&result_content, 1200)
                        ));
                    }
                    "image" if detailed => parts.push("[image omitted]".to_string()),
                    "document" if detailed => parts.push("[document omitted]".to_string()),
                    _ => {}
                }
            } else if let Some(s) = item.as_str() {
                parts.push(s.to_string());
            }
        }
        return parts.join("\n");
    }
    truncate_value(content, 1200)
}

fn summarize_non_dialog_record(value: &Value) -> String {
    let record_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match record_type {
        "progress" => {
            let ptype = value
                .get("data")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let hook = value
                .get("data")
                .and_then(|d| d.get("hookName"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let cmd = value
                .get("data")
                .and_then(|d| d.get("command"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut s = format!("progress:{ptype}");
            if !hook.is_empty() {
                s.push_str(&format!(" hook={hook}"));
            }
            if !cmd.is_empty() {
                s.push_str(&format!(" cmd={}", ellipsize(cmd, 120)));
            }
            s
        }
        "system" => {
            let subtype = value
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            format!("system:{subtype}")
        }
        "queue-operation" => {
            let op = value
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            format!("queue-operation:{op}")
        }
        "file-history-snapshot" => "file-history-snapshot".to_string(),
        _ => truncate_value(value, 300),
    }
}

fn search_sessions(sessions: &[Session], args: &SearchArgs) -> Result<Vec<SearchHit>> {
    let regex = if matches!(args.mode, SearchMode::Regex) {
        Some(
            RegexBuilder::new(&args.query)
                .case_insensitive(!args.case_sensitive)
                .build()
                .with_context(|| format!("Invalid regex: {}", args.query))?,
        )
    } else {
        None
    };

    let query_normalized = if args.case_sensitive {
        args.query.clone()
    } else {
        args.query.to_lowercase()
    };
    let query_tokens = query_normalized
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();

    let mut hits = Vec::new();
    for session in sessions {
        let parsed = parse_session_events(&session.path, false)?;
        for event in parsed.events {
            if args.speaker != SpeakerFilter::Both {
                if args.speaker == SpeakerFilter::User && event.role != "user" {
                    continue;
                }
                if args.speaker == SpeakerFilter::Assistant && event.role != "assistant" {
                    continue;
                }
            }

            let haystack = if args.case_sensitive {
                event.content.clone()
            } else {
                event.content.to_lowercase()
            };
            let (matched, relevance) = match args.mode {
                SearchMode::Exact => {
                    if haystack.contains(&query_normalized) {
                        let count = haystack.matches(&query_normalized).count() as f64;
                        (true, (0.5 + (count * 0.1)).min(1.0))
                    } else {
                        (false, 0.0)
                    }
                }
                SearchMode::Regex => {
                    let re = regex.as_ref().expect("regex compiled");
                    let m = re.find(&event.content);
                    if m.is_some() {
                        (true, 0.8)
                    } else {
                        (false, 0.0)
                    }
                }
                SearchMode::Smart => {
                    let mut score = 0.0;
                    if haystack.contains(&query_normalized) {
                        score += 0.6;
                    }
                    if !query_tokens.is_empty() {
                        let overlap = query_tokens
                            .iter()
                            .filter(|tok| haystack.contains(**tok))
                            .count() as f64;
                        score += (overlap / query_tokens.len() as f64) * 0.4;
                    }
                    (score > 0.15, score.min(1.0))
                }
            };

            if matched {
                let preview = build_context_preview(
                    &event.content,
                    &args.query,
                    args.context_chars,
                    args.case_sensitive,
                );
                hits.push(SearchHit {
                    session_id: session.id.clone(),
                    project: session.project.clone(),
                    path: session.path.clone(),
                    speaker: event.role,
                    timestamp: event.timestamp,
                    relevance,
                    preview,
                });
            }
        }
    }

    hits.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(hits)
}

fn select_sessions_for_export(sessions: &[Session], args: &ExportArgs) -> Result<Vec<Session>> {
    let mut selected_by_id: HashSet<String> = HashSet::new();
    let mut selected = Vec::new();
    let mut push_unique = |s: &Session| {
        if selected_by_id.insert(s.id.clone()) {
            selected.push(s.clone());
        }
    };

    for sid in &args.sessions {
        let found = sessions.iter().find(|s| &s.id == sid || &s.id_short == sid);
        if let Some(s) = found {
            push_unique(s);
        } else {
            bail!("Session not found: {}", sid);
        }
    }

    for idx in &args.indices {
        if *idx == 0 {
            bail!("--index uses 1-based indexing; got 0");
        }
        let s = sessions
            .get(idx - 1)
            .ok_or_else(|| anyhow!("Invalid index {}", idx))?;
        push_unique(s);
    }

    if let Some(recent) = args.recent {
        for s in sessions.iter().take(recent) {
            push_unique(s);
        }
    }

    if args.all {
        for s in sessions {
            push_unique(s);
        }
    }

    if let Some(query) = &args.search {
        let search_args = SearchArgs {
            query: query.clone(),
            mode: SearchMode::Smart,
            speaker: SpeakerFilter::Both,
            case_sensitive: false,
            max_results: usize::MAX,
            context_chars: 150,
        };
        let hits = search_sessions(sessions, &search_args)?;
        let hit_sessions: HashSet<String> = hits.into_iter().map(|h| h.session_id).collect();
        for s in sessions {
            if hit_sessions.contains(&s.id) {
                push_unique(s);
            }
        }
    }

    if selected.is_empty() {
        bail!(
            "No selection flags provided. Use one of: --session, --index, --recent, --all, --search"
        );
    }

    selected.sort_by_key(|s| s.index);
    Ok(selected)
}

#[derive(Debug, Clone, Serialize)]
struct ExportDocument {
    session_id: String,
    session_short: String,
    project: String,
    source_path: PathBuf,
    modified_iso: String,
    event_count: usize,
    events: Vec<NormalizedEvent>,
}

fn build_export_document(session: &Session, events: &[NormalizedEvent]) -> ExportDocument {
    ExportDocument {
        session_id: session.id.clone(),
        session_short: session.id_short.clone(),
        project: session.project.clone(),
        source_path: session.path.clone(),
        modified_iso: session.modified_iso.clone(),
        event_count: events.len(),
        events: events.to_vec(),
    }
}

fn write_single_export(
    output_dir: &Path,
    doc: &ExportDocument,
    format: ExportFormat,
) -> Result<PathBuf> {
    let date = doc.modified_iso.split('T').next().unwrap_or("unknown-date");
    let ext = match format {
        ExportFormat::Markdown => "md",
        ExportFormat::Json => "json",
        ExportFormat::Html => "html",
    };
    let filename = format!("cc-convo-{date}-{}.{}", doc.session_short, ext);
    let path = output_dir.join(filename);
    let body = match format {
        ExportFormat::Markdown => render_markdown(std::slice::from_ref(doc)),
        ExportFormat::Json => serde_json::to_string_pretty(doc)?,
        ExportFormat::Html => render_html(std::slice::from_ref(doc)),
    };
    fs::write(&path, body)?;
    Ok(path)
}

fn write_bundle_export(
    output_dir: &Path,
    docs: &[ExportDocument],
    format: ExportFormat,
) -> Result<PathBuf> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let date = now.split('T').next().unwrap_or("unknown-date");
    let ext = match format {
        ExportFormat::Markdown => "md",
        ExportFormat::Json => "json",
        ExportFormat::Html => "html",
    };
    let path = output_dir.join(format!("cc-convo-bundle-{date}.{ext}"));
    let body = match format {
        ExportFormat::Markdown => render_markdown(docs),
        ExportFormat::Json => serde_json::to_string_pretty(docs)?,
        ExportFormat::Html => render_html(docs),
    };
    fs::write(&path, body)?;
    Ok(path)
}

fn render_markdown(docs: &[ExportDocument]) -> String {
    let mut out = String::new();
    for (di, doc) in docs.iter().enumerate() {
        if di > 0 {
            out.push_str("\n\n---\n\n");
        }
        out.push_str("# cc-convo export\n\n");
        out.push_str(&format!("- Session: `{}`\n", doc.session_id));
        out.push_str(&format!("- Project: `{}`\n", doc.project));
        out.push_str(&format!("- Modified: `{}`\n", doc.modified_iso));
        out.push_str(&format!("- Source: `{}`\n", doc.source_path.display()));
        out.push_str(&format!("- Events: `{}`\n\n", doc.event_count));
        for event in &doc.events {
            out.push_str(&format!(
                "## [{}] {}\n\n",
                event.role,
                event.timestamp.clone().unwrap_or_else(|| "-".to_string())
            ));
            out.push_str(&event.content);
            out.push_str("\n\n");
        }
    }
    out
}

fn render_html(docs: &[ExportDocument]) -> String {
    let mut out = String::new();
    out.push_str(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>cc-convo export</title>",
    );
    out.push_str("<style>body{font-family:ui-sans-serif,system-ui;margin:2rem;background:#f7f8fa;color:#1e2430} .card{background:#fff;border-radius:12px;padding:16px 20px;margin:0 0 16px 0;box-shadow:0 1px 2px rgba(0,0,0,.06)} .meta{color:#5c667a;font-size:.92rem} pre{white-space:pre-wrap;word-break:break-word;margin:0} h1,h2{margin:.2rem 0 .8rem} </style>");
    out.push_str("</head><body><h1>cc-convo export</h1>");
    for doc in docs {
        out.push_str("<div class=\"card\">");
        out.push_str(&format!(
            "<h2>{}</h2><div class=\"meta\">project={} modified={} source={} events={}</div>",
            html_escape(&doc.session_id),
            html_escape(&doc.project),
            html_escape(&doc.modified_iso),
            html_escape(&doc.source_path.display().to_string()),
            doc.event_count
        ));
        out.push_str("</div>");
        for event in &doc.events {
            out.push_str("<div class=\"card\">");
            out.push_str(&format!(
                "<h2>[{}] {}</h2><pre>{}</pre>",
                html_escape(&event.role),
                html_escape(&event.timestamp.clone().unwrap_or_else(|| "-".to_string())),
                html_escape(&event.content)
            ));
            out.push_str("</div>");
        }
    }
    out.push_str("</body></html>");
    out
}

fn resolve_session_target<'a>(sessions: &'a [Session], target: &str) -> Result<&'a Session> {
    if let Ok(index) = target.parse::<usize>() {
        if index == 0 {
            bail!("Session index is 1-based; got 0");
        }
        return sessions
            .get(index - 1)
            .ok_or_else(|| anyhow!("Invalid session index {}", index));
    }
    sessions
        .iter()
        .find(|s| s.id == target || s.id_short == target)
        .ok_or_else(|| anyhow!("Session not found: {}", target))
}

fn short_id(full: &str) -> String {
    full.chars().take(8).collect()
}

fn build_context_preview(
    text: &str,
    query: &str,
    context_chars: usize,
    case_sensitive: bool,
) -> String {
    let hay = if case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    if let Some(pos) = hay.find(&needle) {
        let start = pos.saturating_sub(context_chars);
        let end = (pos + needle.len() + context_chars).min(text.len());
        let slice = text.get(start..end).unwrap_or(text);
        let mut preview = String::new();
        if start > 0 {
            preview.push_str("...");
        }
        preview.push_str(slice);
        if end < text.len() {
            preview.push_str("...");
        }
        preview.replace('\n', " ")
    } else {
        ellipsize(&text.replace('\n', " "), context_chars * 2)
    }
}

fn clean_preview(s: &str) -> String {
    ellipsize(&s.replace('\n', " ").trim().to_string(), 140)
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn truncate_value(v: &Value, max: usize) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "<invalid-json>".to_string());
    ellipsize(&s, max)
}

fn print_sessions_table(items: &[SessionSummary], with_preview: bool) {
    println!("{}", style("Sessions").bold().cyan());
    if items.is_empty() {
        println!("No sessions found.");
        return;
    }
    if with_preview {
        println!(
            "{:<5} {:<10} {:<36} {:<26} {:<20} {:>8} {:>6} {:>6} {:>6}  {}",
            "Idx",
            "ShortId",
            "SessionId",
            "Project",
            "Modified",
            "SizeKB",
            "User",
            "Asst",
            "Other",
            "Preview"
        );
    } else {
        println!(
            "{:<5} {:<10} {:<36} {:<26} {:<20} {:>8} {:>6} {:>6} {:>6}",
            "Idx", "ShortId", "SessionId", "Project", "Modified", "SizeKB", "User", "Asst", "Other"
        );
    }
    for s in items {
        let modified = &s.session.modified_iso;
        if with_preview {
            println!(
                "{:<5} {:<10} {:<36} {:<26} {:<20} {:>8.1} {:>6} {:>6} {:>6}  {}",
                s.session.index,
                s.session.id_short,
                s.session.id,
                ellipsize(&s.session.project, 26),
                modified,
                s.session.size_bytes as f64 / 1024.0,
                s.user_messages,
                s.assistant_messages,
                s.other_records,
                s.preview.clone().unwrap_or_else(|| "-".to_string())
            );
        } else {
            println!(
                "{:<5} {:<10} {:<36} {:<26} {:<20} {:>8.1} {:>6} {:>6} {:>6}",
                s.session.index,
                s.session.id_short,
                s.session.id,
                ellipsize(&s.session.project, 26),
                modified,
                s.session.size_bytes as f64 / 1024.0,
                s.user_messages,
                s.assistant_messages,
                s.other_records
            );
        }
    }
}

fn top_n_sorted_map(map: HashMap<String, u64>, n: usize) -> Vec<(String, u64)> {
    let mut vec = map.into_iter().collect::<Vec<_>>();
    vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    vec.into_iter().take(n).collect()
}

fn print_ranked_map(title: &str, items: &[(String, u64)]) {
    println!("{}", style(title).bold());
    if items.is_empty() {
        println!("  (none)");
        return;
    }
    for (k, v) in items {
        println!("  {:>7}  {}", v, k);
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value)?;
    writeln!(lock)?;
    Ok(())
}

fn read_raw_lines(path: &Path) -> Result<Vec<String>> {
    let f = File::open(path)?;
    let reader = BufReader::new(f);
    let mut out = Vec::new();
    for line in reader.lines() {
        out.push(line?);
    }
    Ok(out)
}

fn time_window_from_global(global: &GlobalArgs) -> Result<TimeWindow> {
    let since = if let Some(hours) = global.since_hours {
        if hours == 0 {
            bail!("--since-hours must be > 0");
        }
        Some(Utc::now() - chrono::Duration::hours(hours as i64))
    } else if let Some(days) = global.since_days {
        if days == 0 {
            bail!("--since-days must be > 0");
        }
        Some(Utc::now() - chrono::Duration::days(days as i64))
    } else {
        None
    };

    let until = if let Some(raw) = &global.until {
        let dt = DateTime::parse_from_rfc3339(raw)
            .with_context(|| format!("Invalid --until timestamp: {raw}"))?;
        Some(dt.with_timezone(&Utc))
    } else {
        None
    };
    Ok(TimeWindow { since, until })
}

fn expand_tilde_path(input: &str) -> Result<PathBuf> {
    if input == "~" {
        let home = std::env::var("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home));
    }
    if let Some(rest) = input.strip_prefix("~/") {
        let home = std::env::var("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home).join(rest));
    }
    Ok(PathBuf::from(input))
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), nanos))
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let body = lines.join("\n");
        fs::write(path, body).expect("write jsonl");
    }

    #[test]
    fn parse_default_mode_extracts_text_only() {
        let dir = unique_temp_path("cc-convo-test-default");
        fs::create_dir_all(&dir).expect("create temp dir");
        let file = dir.join("session.jsonl");
        write_jsonl(
            &file,
            &[
                r#"{"type":"user","timestamp":"2026-02-21T00:00:00Z","message":{"content":[{"type":"text","text":"hello user"}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-02-21T00:00:01Z","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"tool_use","name":"x","input":{"a":1}},{"type":"text","text":"hello assistant"}]}}"#,
                r#"{"type":"progress","timestamp":"2026-02-21T00:00:02Z","data":{"type":"tool","hookName":"h"}}"#,
            ],
        );

        let parsed = parse_session_events(&file, false).expect("parse");
        assert_eq!(parsed.parse_errors, 0);
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.events[0].role, "user");
        assert_eq!(parsed.events[0].content, "hello user");
        assert_eq!(parsed.events[1].role, "assistant");
        assert_eq!(parsed.events[1].content, "hello assistant");

        fs::remove_file(&file).expect("cleanup file");
        fs::remove_dir_all(&dir).expect("cleanup dir");
    }

    #[test]
    fn parse_detailed_mode_includes_operational_blocks() {
        let dir = unique_temp_path("cc-convo-test-detailed");
        fs::create_dir_all(&dir).expect("create temp dir");
        let file = dir.join("session.jsonl");
        write_jsonl(
            &file,
            &[
                r#"{"type":"assistant","timestamp":"2026-02-21T00:00:01Z","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"tool_use","name":"x","input":{"a":1}},{"type":"tool_result","tool_use_id":"abc","content":"done"},{"type":"text","text":"visible"}]}}"#,
                r#"{"type":"progress","timestamp":"2026-02-21T00:00:02Z","data":{"type":"tool","hookName":"h"}}"#,
            ],
        );

        let parsed = parse_session_events(&file, true).expect("parse");
        assert_eq!(parsed.parse_errors, 0);
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.events[0].role, "assistant");
        assert!(parsed.events[0].content.contains("[thinking]"));
        assert!(parsed.events[0].content.contains("[tool_use] x"));
        assert!(parsed.events[0].content.contains("[tool_result] abc"));
        assert!(parsed.events[0].content.contains("visible"));
        assert_eq!(parsed.events[1].role, "progress");
        assert!(parsed.events[1].content.contains("progress:tool"));

        fs::remove_file(&file).expect("cleanup file");
        fs::remove_dir_all(&dir).expect("cleanup dir");
    }

    #[test]
    fn build_context_preview_falls_back_to_ellipsized_text() {
        let text = "alpha beta gamma delta epsilon";
        let preview = build_context_preview(text, "notfound", 5, false);
        assert!(preview.contains("..."));
        assert!(preview.len() <= 13);
    }
}
