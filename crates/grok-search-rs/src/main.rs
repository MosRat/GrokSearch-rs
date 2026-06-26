use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use grok_search_config::{self as config, AuthMode, Config, InitOutcome};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // CLI shim: handle --version, --init before MCP server mode.
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args
        .iter()
        .any(|a| a == "--version" || a == "-V" || a == "-v")
    {
        println!("grok-search-rs {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|a| a == "init" || a == "--init") {
        return run_init();
    }

    if args.first().map(String::as_str) == Some("login") {
        let cfg = Config::load();
        return run_login(&cfg).await;
    }

    if args.first().map(String::as_str) == Some("status") {
        let cfg = Config::load();
        return run_status(&cfg);
    }

    if args.first().map(String::as_str) == Some("logout") {
        let cfg = Config::load();
        return run_logout(&cfg);
    }

    let cfg = Config::load();

    // Detect interactive run with missing credentials and print a friendly
    // onboarding guide instead of a cryptic error. MCP clients always pipe
    // stdio, so a TTY here means the user ran the binary directly.
    if cfg.grok_auth_mode == AuthMode::ApiKey
        && cfg.grok_api_key.is_none()
        && std::io::stdin().is_terminal()
    {
        print_setup_guide();
        return Ok(());
    }

    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await;
    let service = grok_search_runtime::new_with_http(cfg, http, proxy_diagnostics)?;
    grok_search_mcp::run_stdio(service).await?;
    Ok(())
}

async fn run_login(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let store = grok_search_auth::oauth::login::login(&path, true).await?;
    println!("Login successful.");
    println!("Auth file: {}", path.display());
    if let Some(exp) = grok_search_auth::oauth::token_store::jwt_exp(&store.access_token) {
        println!("Access token expires at unix time: {exp}");
    }

    Ok(())
}

const INIT_BEGIN: &str = "# BEGIN grok-search-rs init";
const INIT_END: &str = "# END grok-search-rs init";

enum ClaudeInitOutcome {
    Configured,
    CommandUnavailable(String),
    CommandFailed { command: String, detail: String },
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

fn configure_claude_code(explicit_config: Option<&Path>) -> anyhow::Result<ClaudeInitOutcome> {
    let payload = mcp_server_json(explicit_config);
    let command = format!(
        "claude mcp add-json grok-search-rs --scope user {}",
        shell_quote(&payload)
    );
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

fn write_agent_snippets(
    global_config: &Path,
    explicit_config: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let dir = global_config
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("agent-snippets");
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

fn run_status(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let status = grok_search_auth::oauth::token_store::auth_status(&path);
    println!("grok-search-rs OAuth status");
    println!("  Auth file: {}", status.path.display());
    println!(
        "  Authenticated: {}",
        if status.authenticated { "yes" } else { "no" }
    );
    println!(
        "  Refresh token: {}",
        if status.refresh_token_present {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "  Access expires at: {}",
        status
            .access_expires_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "  Base URL: {}",
        status.base_url.unwrap_or_else(|| "unknown".to_string())
    );
    Ok(())
}

fn run_logout(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let removed = grok_search_auth::oauth::token_store::delete_token_store(&path)?;
    if removed {
        println!("Removed OAuth token file: {}", path.display());
    } else {
        println!("No OAuth token file found: {}", path.display());
    }
    Ok(())
}

fn resolve_auth_path(cfg: &Config) -> anyhow::Result<std::path::PathBuf> {
    cfg.grok_auth_file
        .clone()
        .or_else(config::auth_path)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve OAuth auth path; set GROK_SEARCH_AUTH_FILE"))
}

/// Scaffold the shared config and keep thin MCP client entries in sync.
fn run_init() -> anyhow::Result<()> {
    let path = config::config_path().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot resolve config path: set GROK_SEARCH_CONFIG to an explicit file path, \
             or ensure HOME (Unix / Git Bash) or USERPROFILE (Windows) is set"
        )
    })?;
    match config::write_template(&path)? {
        InitOutcome::Created => {
            println!("created global config template: {}", path.display());
            println!("  edit it and uncomment the providers, models, and keys you need.");
        }
        InitOutcome::AlreadyExists => {
            println!("global config already exists: {}", path.display());
            println!("  leaving it untouched; use it for shared providers, models, and keys.");
        }
    }

    let explicit_config = std::env::var_os("GROK_SEARCH_CONFIG").is_some();
    let forwarded_config = explicit_config.then_some(path.as_path());

    if let Some(codex_path) = codex_config_path() {
        upsert_codex_config(&codex_path, forwarded_config)?;
        println!("updated Codex MCP config: {}", codex_path.display());
    } else {
        println!("skipped Codex MCP config: cannot resolve HOME or CODEX_HOME");
    }

    match configure_claude_code(forwarded_config) {
        Ok(ClaudeInitOutcome::Configured) => {
            println!("updated Claude Code MCP config via claude mcp add-json.");
        }
        Ok(ClaudeInitOutcome::CommandUnavailable(command)) => {
            println!("Claude Code CLI not found; run this when claude is available:");
            println!("  {command}");
        }
        Ok(ClaudeInitOutcome::CommandFailed { command, detail }) => {
            println!("Claude Code CLI did not accept the MCP config: {detail}");
            println!("  retry manually: {command}");
        }
        Err(err) => return Err(err),
    }

    let snippets_dir = write_agent_snippets(&path, forwarded_config)?;
    println!(
        "wrote thin MCP snippets for other agents: {}",
        snippets_dir.display()
    );
    println!(
        "agent configs stay thin; keep provider URLs, models, and API keys in the global config."
    );
    Ok(())
}
fn print_setup_guide() {
    let mut guide = String::from(
        r#"grok-search-rs is an MCP server. It speaks JSON-RPC over stdio and
should be launched by an MCP client (Claude Code, Codex CLI, Gemini CLI,
Cursor, VS Code, Windsurf, ...), not run directly.

Required keys
  GROK_SEARCH_API_KEY   xAI / Grok-compatible key   (https://x.ai/api)
  TAVILY_API_KEY        Tavily fetch + map          (https://tavily.com)
  FIRECRAWL_API_KEY     optional fetch fallback     (https://firecrawl.dev)

OAuth alternative
  grok-search-rs login
  Set GROK_SEARCH_AUTH_MODE=oauth in your MCP env or config.
  OAuth mode reuses Hermes' xAI client_id and may carry account / terms risk.

Recommended setup
  grok-search-rs init
  Fill the global config once, then keep each MCP client entry thin:
  {"type":"stdio","command":"grok-search-rs"}

"#,
    );

    // Hint the global config path only when the file is genuinely missing.
    if let Some(path) = config::config_path() {
        if !path.exists() {
            guide.push_str(&format!(
                r#"Tip: set keys once for every MCP client
  grok-search-rs init                    # scaffold config + thin agent entries
  $EDITOR {}    # uncomment and fill

"#,
                path.display()
            ));
        }
    }

    guide.push_str(
        r#"Docs:    https://github.com/MosRat/GrokSearch-rs#readme
Issues:  https://github.com/MosRat/GrokSearch-rs/issues
"#,
    );

    let stdout = std::io::stdout();
    let _ = stdout.lock().write_all(guide.as_bytes());
}

#[cfg(test)]
mod init_tests {
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
}
