use std::path::{Path, PathBuf};
use std::process::Command;

use grok_search_config::{self as config, InitOutcome};

const INIT_BEGIN: &str = "# BEGIN grok-search-rs init";
const INIT_END: &str = "# END grok-search-rs init";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitTarget {
    All,
    Codex,
    ClaudeCode,
    Snippets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitOptions {
    pub target: InitTarget,
    pub dry_run: bool,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            target: InitTarget::All,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    pub messages: Vec<String>,
}

impl InitReport {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    fn push(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());
    }
}

pub fn run_init(options: InitOptions) -> anyhow::Result<InitReport> {
    let path = config::config_path().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot resolve config path: set GROK_SEARCH_CONFIG to an explicit file path, \
             or ensure HOME (Unix / Git Bash) or USERPROFILE (Windows) is set"
        )
    })?;
    let explicit_config = std::env::var_os("GROK_SEARCH_CONFIG").is_some();
    let forwarded_config = explicit_config.then_some(path.as_path());
    let mut report = InitReport::new();

    if options.dry_run {
        if path.exists() {
            report.push(format!(
                "would keep existing global config: {}",
                path.display()
            ));
        } else {
            report.push(format!(
                "would create global config template: {}",
                path.display()
            ));
        }
    } else {
        match config::write_template(&path)? {
            InitOutcome::Created => {
                report.push(format!(
                    "created global config template: {}",
                    path.display()
                ));
                report.push(
                    "  edit it and uncomment the providers, models, and keys you need.".to_string(),
                );
            }
            InitOutcome::AlreadyExists => {
                report.push(format!("global config already exists: {}", path.display()));
                report.push(
                    "  leaving it untouched; use it for shared providers, models, and keys."
                        .to_string(),
                );
            }
        }
    }

    if matches!(options.target, InitTarget::All | InitTarget::Codex) {
        init_codex(forwarded_config, options.dry_run, &mut report)?;
    }
    if matches!(options.target, InitTarget::All | InitTarget::ClaudeCode) {
        init_claude_code(forwarded_config, options.dry_run, &mut report)?;
    }
    if matches!(options.target, InitTarget::All | InitTarget::Snippets) {
        init_snippets(&path, forwarded_config, options.dry_run, &mut report)?;
    }

    report.push(
        "agent configs stay thin; keep provider URLs, models, and API keys in the global config.",
    );
    Ok(report)
}

fn init_codex(
    explicit_config: Option<&Path>,
    dry_run: bool,
    report: &mut InitReport,
) -> anyhow::Result<()> {
    if let Some(codex_path) = codex_config_path() {
        if dry_run {
            report.push(format!(
                "would update Codex MCP config: {}",
                codex_path.display()
            ));
        } else {
            upsert_codex_config(&codex_path, explicit_config)?;
            report.push(format!(
                "updated Codex MCP config: {}",
                codex_path.display()
            ));
        }
    } else {
        report.push("skipped Codex MCP config: cannot resolve HOME or CODEX_HOME");
    }
    Ok(())
}

fn init_claude_code(
    explicit_config: Option<&Path>,
    dry_run: bool,
    report: &mut InitReport,
) -> anyhow::Result<()> {
    if dry_run {
        report.push(format!(
            "would run Claude Code command: {}",
            claude_command(explicit_config)
        ));
        return Ok(());
    }

    match configure_claude_code(explicit_config) {
        Ok(ClaudeInitOutcome::Configured) => {
            report.push("updated Claude Code MCP config via claude mcp add-json.");
        }
        Ok(ClaudeInitOutcome::CommandUnavailable(command)) => {
            report.push("Claude Code CLI not found; run this when claude is available:");
            report.push(format!("  {command}"));
        }
        Ok(ClaudeInitOutcome::CommandFailed { command, detail }) => {
            report.push(format!(
                "Claude Code CLI did not accept the MCP config: {detail}"
            ));
            report.push(format!("  retry manually: {command}"));
        }
        Err(err) => return Err(err),
    }
    Ok(())
}

fn init_snippets(
    global_config: &Path,
    explicit_config: Option<&Path>,
    dry_run: bool,
    report: &mut InitReport,
) -> anyhow::Result<()> {
    let snippets_dir = snippets_dir(global_config);
    if dry_run {
        report.push(format!(
            "would write thin MCP snippets for other agents: {}",
            snippets_dir.display()
        ));
    } else {
        write_agent_snippets(global_config, explicit_config)?;
        report.push(format!(
            "wrote thin MCP snippets for other agents: {}",
            snippets_dir.display()
        ));
    }
    Ok(())
}

fn codex_config_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home).join("config.toml"));
    }
    home_dir().map(|home| home.join(".codex").join("config.toml"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
}

fn upsert_codex_config(path: &Path, explicit_config: Option<&Path>) -> anyhow::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let cleaned = remove_codex_init_block(&remove_codex_server_sections(&existing));
    let mut next = cleaned.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(&codex_toml_block(explicit_config));
    next.push('\n');

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, next)?;
    Ok(())
}

fn codex_toml_block(explicit_config: Option<&Path>) -> String {
    let mut block = String::from(
        r#"# BEGIN grok-search-rs init
[mcp_servers.grok-search-rs]
type = "stdio"
command = "grok-search-rs"
"#,
    );
    if let Some(path) = explicit_config {
        block.push_str("\n[mcp_servers.grok-search-rs.env]\n");
        block.push_str("GROK_SEARCH_CONFIG = ");
        block.push_str(&toml_string(&path.display().to_string()));
        block.push('\n');
    }
    block.push_str("# END grok-search-rs init");
    block
}

fn remove_codex_init_block(input: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in input.lines() {
        if line.trim() == INIT_BEGIN {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == INIT_END {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn remove_codex_server_sections(input: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in input.lines() {
        if let Some(header) = toml_header(line) {
            skipping = header == "mcp_servers.grok-search-rs"
                || header.starts_with("mcp_servers.grok-search-rs.");
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn toml_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') || trimmed.starts_with("[[") {
        return None;
    }
    Some(
        trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim()
            .to_string(),
    )
}

enum ClaudeInitOutcome {
    Configured,
    CommandUnavailable(String),
    CommandFailed { command: String, detail: String },
}

fn configure_claude_code(explicit_config: Option<&Path>) -> anyhow::Result<ClaudeInitOutcome> {
    let payload = mcp_server_json(explicit_config);
    let command = claude_command(explicit_config);
    let output = Command::new("claude")
        .args([
            "mcp",
            "add-json",
            "grok-search-rs",
            "--scope",
            "user",
            &payload,
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(ClaudeInitOutcome::Configured),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("exit status {}", output.status)
            };
            Ok(ClaudeInitOutcome::CommandFailed { command, detail })
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(ClaudeInitOutcome::CommandUnavailable(command))
        }
        Err(err) => Err(err.into()),
    }
}

fn claude_command(explicit_config: Option<&Path>) -> String {
    format!(
        "claude mcp add-json grok-search-rs --scope user {}",
        shell_quote(&mcp_server_json(explicit_config))
    )
}

fn write_agent_snippets(
    global_config: &Path,
    explicit_config: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let dir = snippets_dir(global_config);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("codex.toml"), codex_toml_block(explicit_config))?;
    std::fs::write(dir.join("mcp.json"), generic_mcp_json(explicit_config))?;
    std::fs::write(
        dir.join("claude-code.json"),
        mcp_server_json(explicit_config),
    )?;
    std::fs::write(dir.join("cursor.json"), generic_mcp_json(explicit_config))?;
    std::fs::write(dir.join("vscode.json"), generic_mcp_json(explicit_config))?;
    std::fs::write(dir.join("windsurf.json"), generic_mcp_json(explicit_config))?;
    Ok(dir)
}

fn snippets_dir(global_config: &Path) -> PathBuf {
    global_config
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("agent-snippets")
}

fn generic_mcp_json(explicit_config: Option<&Path>) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"grok-search-rs\": {}\n  }}\n}}\n",
        mcp_server_json(explicit_config)
    )
}

fn mcp_server_json(explicit_config: Option<&Path>) -> String {
    let mut body = String::from("{\"type\":\"stdio\",\"command\":\"grok-search-rs\"");
    if let Some(path) = explicit_config {
        body.push_str(",\"env\":{\"GROK_SEARCH_CONFIG\":");
        body.push_str(&json_string(&path.display().to_string()));
        body.push('}');
    }
    body.push('}');
    body
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn shell_quote(value: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_codex_config_omits_env_for_default_global_config() {
        let block = codex_toml_block(None);

        assert!(block.contains("[mcp_servers.grok-search-rs]"));
        assert!(block.contains("command = \"grok-search-rs\""));
        assert!(!block.contains("[mcp_servers.grok-search-rs.env]"));
        assert!(!block.contains("GROK_SEARCH_API_KEY"));
        assert!(!block.contains("TAVILY_API_KEY"));
    }

    #[test]
    fn thin_codex_config_forwards_explicit_global_config_only() {
        let path = Path::new(r"C:\Users\alice\custom config.toml");
        let block = codex_toml_block(Some(path));

        assert!(block.contains("[mcp_servers.grok-search-rs.env]"));
        assert!(block.contains(r#"GROK_SEARCH_CONFIG = "C:\\Users\\alice\\custom config.toml""#));
        assert!(!block.contains("GROK_SEARCH_MODEL"));
    }

    #[test]
    fn removes_old_codex_sections_without_touching_other_tables() {
        let input = r#"
[profile.default]
model = "gpt"

[mcp_servers.grok-search-rs]
type = "stdio"
command = "old"

[mcp_servers.grok-search-rs.env]
GROK_SEARCH_API_KEY = "secret"

[mcp_servers.grok-search-rs.extra]
note = "legacy"

[mcp_servers.other]
command = "other"
"#;

        let cleaned = remove_codex_server_sections(input);

        assert!(!cleaned.contains("[mcp_servers.grok-search-rs]"));
        assert!(!cleaned.contains("GROK_SEARCH_API_KEY"));
        assert!(!cleaned.contains("legacy"));
        assert!(cleaned.contains("[profile.default]"));
        assert!(cleaned.contains("[mcp_servers.other]"));
    }

    #[test]
    fn removes_marked_init_block_idempotently() {
        let input = r#"
before = true
# BEGIN grok-search-rs init
[mcp_servers.grok-search-rs]
command = "old"
# END grok-search-rs init
after = true
"#;

        let cleaned = remove_codex_init_block(input);

        assert!(cleaned.contains("before = true"));
        assert!(cleaned.contains("after = true"));
        assert!(!cleaned.contains("command = \"old\""));
    }

    #[test]
    fn generic_mcp_json_stays_thin() {
        let json = generic_mcp_json(None);

        assert!(json.contains("\"mcpServers\""));
        assert!(json.contains("\"command\":\"grok-search-rs\""));
        assert!(!json.contains("\"env\""));
    }

    #[test]
    fn dry_run_does_not_create_files() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        std::env::set_var("GROK_SEARCH_CONFIG", &config_path);
        std::env::set_var("CODEX_HOME", temp.path().join(".codex"));

        let report = run_init(InitOptions {
            target: InitTarget::All,
            dry_run: true,
        })
        .unwrap();

        assert!(report.messages.iter().any(|m| m.contains("would create")));
        assert!(!config_path.exists());

        std::env::remove_var("GROK_SEARCH_CONFIG");
        std::env::remove_var("CODEX_HOME");
    }
}
