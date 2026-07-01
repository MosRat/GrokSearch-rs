use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const DEFAULT_MCP_SERVICE_NAME: &str = "grok-search-rs-http-mcp";
const DEFAULT_WINDOWS_TASK_NAME: &str = "GrokSearchRsHttpMcp";
const DEFAULT_MACOS_LABEL: &str = "dev.grok-search-rs.http-mcp";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServiceOptions {
    pub name: Option<String>,
    pub command: McpServiceCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServiceCommand {
    Install(McpServiceInstallOptions),
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServiceInstallOptions {
    pub bind: SocketAddr,
    pub path: String,
    pub allow_origin: Option<String>,
    pub install_dir: Option<PathBuf>,
    pub auth_token_configured: bool,
    pub no_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServiceReport {
    pub messages: Vec<String>,
}

impl McpServiceReport {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    fn push(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServicePlatform {
    Windows,
    Linux,
    Macos,
}

impl ServicePlatform {
    fn current() -> anyhow::Result<Self> {
        if cfg!(windows) {
            Ok(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Ok(Self::Macos)
        } else {
            anyhow::bail!(
                "HTTP MCP user service install is not supported on this platform; run `grok-search-rs mcp-http` directly"
            );
        }
    }
}

#[derive(Debug, Clone)]
struct ServiceContext {
    platform: ServicePlatform,
    current_exe: PathBuf,
    home_dir: PathBuf,
    local_app_data: Option<PathBuf>,
    explicit_config: Option<PathBuf>,
}

impl ServiceContext {
    fn current() -> anyhow::Result<Self> {
        let env = std::env::vars_os().collect::<HashMap<OsString, OsString>>();
        let home_dir = env
            .get(&OsString::from("HOME"))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                env.get(&OsString::from("USERPROFILE"))
                    .filter(|value| !value.is_empty())
            })
            .map(PathBuf::from)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot resolve home directory for service files; set HOME or USERPROFILE"
                )
            })?;
        let explicit_config = std::env::var_os("GROK_SEARCH_CONFIG")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let local_app_data = env
            .get(&OsString::from("LOCALAPPDATA"))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Ok(Self {
            platform: ServicePlatform::current()?,
            current_exe: std::env::current_exe()?,
            home_dir,
            local_app_data,
            explicit_config,
        })
    }
}

pub fn run_mcp_service(options: McpServiceOptions) -> anyhow::Result<McpServiceReport> {
    let context = ServiceContext::current()?;
    let runner = SystemCommandRunner;
    let health = TcpHealthChecker;
    run_mcp_service_with(options, &context, &runner, &health)
}

fn run_mcp_service_with(
    options: McpServiceOptions,
    context: &ServiceContext,
    runner: &dyn CommandRunner,
    health: &dyn HealthChecker,
) -> anyhow::Result<McpServiceReport> {
    let service_name = service_name(options.name.as_deref())?;
    let service = ServiceSpec::new(service_name, context)?;
    let mut report = McpServiceReport::new();

    match options.command {
        McpServiceCommand::Install(install) => {
            let normalized_path = install.validate()?;
            let binary = prepare_managed_binary(&install, context, &service, runner, &mut report)?;
            let command = HttpCommandSpec::new(
                binary,
                install.bind,
                normalized_path,
                install.allow_origin.clone(),
                context.explicit_config.clone(),
            );
            let endpoint = command.endpoint();
            install_service(&service, &command, runner, &mut report)?;
            report.push(format!("installed HTTP MCP user service: {}", service.name));
            report.push(format!("endpoint: {endpoint}"));
            report.push(format!("service file: {}", service.location.display()));
            if let Some(config) = &context.explicit_config {
                report.push(format!("forwarded GROK_SEARCH_CONFIG={}", config.display()));
            } else {
                report.push("using default global config path at service runtime");
            }

            if install.no_start {
                report.push("service registered but not started (--no-start)");
            } else {
                start_service(&service, runner, &mut report)?;
                report_health(install.bind, health, &mut report);
            }
        }
        McpServiceCommand::Uninstall => {
            if service_exists(&service, runner)? {
                stop_service_if_running(&service, runner, &mut report)?;
                uninstall_service(&service, runner, &mut report)?;
                report.push(format!(
                    "uninstalled HTTP MCP user service: {}",
                    service.name
                ));
            } else {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
            }
        }
        McpServiceCommand::Start => {
            if service_exists(&service, runner)? {
                start_service(&service, runner, &mut report)?;
                report.push("start requested");
            } else {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
            }
        }
        McpServiceCommand::Stop => {
            if service_exists(&service, runner)? {
                stop_service_if_running(&service, runner, &mut report)?;
                report.push("stop requested");
            } else {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
            }
        }
        McpServiceCommand::Status => {
            report_status(&service, runner, &mut report)?;
        }
    }

    Ok(report)
}

#[derive(Debug, Clone)]
struct ServiceSpec {
    platform: ServicePlatform,
    name: String,
    platform_id: String,
    location: PathBuf,
}

impl ServiceSpec {
    fn new(name: String, context: &ServiceContext) -> anyhow::Result<Self> {
        let platform_id = platform_id(context.platform, &name);
        let location = match context.platform {
            ServicePlatform::Windows => PathBuf::from(format!(r"\{}", platform_id)),
            ServicePlatform::Linux => context
                .home_dir
                .join(".config")
                .join("systemd")
                .join("user")
                .join(format!("{name}.service")),
            ServicePlatform::Macos => context
                .home_dir
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{platform_id}.plist")),
        };
        Ok(Self {
            platform: context.platform,
            name,
            platform_id,
            location,
        })
    }
}

fn prepare_managed_binary(
    install: &McpServiceInstallOptions,
    context: &ServiceContext,
    service: &ServiceSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<PathBuf> {
    let dir = install
        .install_dir
        .clone()
        .unwrap_or_else(|| default_install_dir(context));
    let target = dir.join(managed_binary_name(context.platform));
    let update = should_update_binary(&context.current_exe, &target, runner)?;
    match update {
        BinaryUpdateDecision::Install => {
            copy_binary(&context.current_exe, &target)?;
            report.push(format!("installed service binary: {}", target.display()));
        }
        BinaryUpdateDecision::Upgrade { existing, current } => {
            let was_installed = service_exists(service, runner).unwrap_or(false);
            if was_installed {
                stop_service_if_running(service, runner, report)?;
            }
            copy_binary(&context.current_exe, &target)?;
            report.push(format!(
                "updated service binary: {} ({} -> {})",
                target.display(),
                existing.unwrap_or_else(|| "unknown".to_string()),
                current
            ));
        }
        BinaryUpdateDecision::Keep { existing } => {
            report.push(format!(
                "kept existing service binary: {} ({})",
                target.display(),
                existing.unwrap_or_else(|| "unknown version".to_string())
            ));
        }
    }
    Ok(target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BinaryUpdateDecision {
    Install,
    Upgrade {
        existing: Option<String>,
        current: String,
    },
    Keep {
        existing: Option<String>,
    },
}

fn should_update_binary(
    current_exe: &Path,
    target: &Path,
    runner: &dyn CommandRunner,
) -> anyhow::Result<BinaryUpdateDecision> {
    if !target.exists() {
        return Ok(BinaryUpdateDecision::Install);
    }
    if same_path(current_exe, target) {
        return Ok(BinaryUpdateDecision::Keep {
            existing: Some(CURRENT_VERSION.to_string()),
        });
    }
    let existing = binary_version(target, runner);
    let current = CURRENT_VERSION.to_string();
    let should_upgrade = match existing.as_deref() {
        Some(existing) => compare_versions(&current, existing).is_gt(),
        None => true,
    };
    if should_upgrade {
        Ok(BinaryUpdateDecision::Upgrade { existing, current })
    } else {
        Ok(BinaryUpdateDecision::Keep { existing })
    }
}

fn binary_version(path: &Path, runner: &dyn CommandRunner) -> Option<String> {
    let args = vec!["--version".to_string()];
    let output = runner.run(&path.display().to_string(), &args).ok()?;
    if !output.success {
        return None;
    }
    parse_version_from_output(&output.stdout)
}

fn parse_version_from_output(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .map(|part| part.trim().to_string())
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = version_parts(left);
    let right = version_parts(right);
    for index in 0..left.len().max(right.len()) {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn copy_binary(source: &Path, target: &Path) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension(format!(
        "{}.tmp",
        target
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("bin")
    ));
    std::fs::copy(source, &tmp)?;
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    std::fs::rename(&tmp, target)?;
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn default_install_dir(context: &ServiceContext) -> PathBuf {
    match context.platform {
        ServicePlatform::Windows => context
            .local_app_data
            .clone()
            .unwrap_or_else(|| context.home_dir.join("AppData").join("Local"))
            .join("GrokSearch-rs")
            .join("bin"),
        ServicePlatform::Linux | ServicePlatform::Macos => {
            context.home_dir.join(".local").join("bin")
        }
    }
}

fn managed_binary_name(platform: ServicePlatform) -> &'static str {
    match platform {
        ServicePlatform::Windows => "grok-search-rs.exe",
        ServicePlatform::Linux | ServicePlatform::Macos => "grok-search-rs",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpCommandSpec {
    exe: PathBuf,
    bind: SocketAddr,
    path: String,
    allow_origin: Option<String>,
    explicit_config: Option<PathBuf>,
}

impl HttpCommandSpec {
    fn new(
        exe: PathBuf,
        bind: SocketAddr,
        path: String,
        allow_origin: Option<String>,
        explicit_config: Option<PathBuf>,
    ) -> Self {
        Self {
            exe,
            bind,
            path,
            allow_origin,
            explicit_config,
        }
    }

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "mcp-http".to_string(),
            "--bind".to_string(),
            self.bind.to_string(),
            "--path".to_string(),
            self.path.clone(),
        ];
        if let Some(origin) = &self.allow_origin {
            args.push("--allow-origin".to_string());
            args.push(origin.clone());
        }
        args
    }

    fn endpoint(&self) -> String {
        format!("http://{}{}", host_port_for_url(self.bind), self.path)
    }
}

impl McpServiceInstallOptions {
    fn validate(&self) -> anyhow::Result<String> {
        let normalized_path = validate_service_name_path(&self.path)?;
        if self.bind.port() == 0 {
            anyhow::bail!("HTTP MCP service bind port cannot be 0");
        }
        if !self.bind.ip().is_loopback() && !self.auth_token_configured {
            anyhow::bail!(
                "HTTP MCP bind address {} is not loopback; set GROK_SEARCH_MCP_HTTP_AUTH_TOKEN before installing the service",
                self.bind
            );
        }
        if let Some(origin) = &self.allow_origin {
            validate_origin(origin)?;
        }
        Ok(normalized_path)
    }
}

fn service_name(value: Option<&str>) -> anyhow::Result<String> {
    let value = value.unwrap_or(DEFAULT_MCP_SERVICE_NAME).trim();
    if value.is_empty() {
        anyhow::bail!("HTTP MCP service name cannot be empty");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!(
            "HTTP MCP service name may only contain ASCII letters, digits, '-', '_' and '.'"
        );
    }
    Ok(value.to_string())
}

fn platform_id(platform: ServicePlatform, name: &str) -> String {
    match platform {
        ServicePlatform::Windows if name == DEFAULT_MCP_SERVICE_NAME => {
            DEFAULT_WINDOWS_TASK_NAME.to_string()
        }
        ServicePlatform::Windows => name.to_string(),
        ServicePlatform::Linux => name.to_string(),
        ServicePlatform::Macos if name == DEFAULT_MCP_SERVICE_NAME => {
            DEFAULT_MACOS_LABEL.to_string()
        }
        ServicePlatform::Macos => format!("dev.grok-search-rs.{name}"),
    }
}

fn validate_service_name_path(path: &str) -> anyhow::Result<String> {
    let path = path.trim();
    if path.is_empty() {
        anyhow::bail!("HTTP MCP path cannot be empty");
    }
    if !path.starts_with('/') {
        anyhow::bail!("HTTP MCP path must start with '/'");
    }
    if path.contains('?') || path.contains('#') {
        anyhow::bail!("HTTP MCP path must not include query or fragment");
    }
    if path != "/" && path.ends_with('/') {
        return Ok(path.trim_end_matches('/').to_string());
    }
    Ok(path.to_string())
}

fn validate_origin(origin: &str) -> anyhow::Result<()> {
    let url = url::Url::parse(origin)?;
    if url.scheme() != "http" && url.scheme() != "https" {
        anyhow::bail!("HTTP MCP allow origin must use http or https");
    }
    if url.host_str().is_none() {
        anyhow::bail!("HTTP MCP allow origin must include a host");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("HTTP MCP allow origin must be an origin only, without path/query/fragment");
    }
    Ok(())
}

trait CommandRunner {
    fn run(&self, program: &str, args: &[String]) -> anyhow::Result<CommandOutput>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[String]) -> anyhow::Result<CommandOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

trait HealthChecker {
    fn check(&self, bind: SocketAddr, timeout: Duration) -> anyhow::Result<()>;
}

struct TcpHealthChecker;

impl HealthChecker for TcpHealthChecker {
    fn check(&self, bind: SocketAddr, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        let target = health_addr(bind);
        let host = host_for_http_header(target.ip(), target.port());
        let request = format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");

        let mut last_error: Option<anyhow::Error> = None;
        while Instant::now() < deadline {
            match TcpStream::connect_timeout(&target, Duration::from_millis(500)) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    if let Err(err) = stream.write_all(request.as_bytes()) {
                        last_error = Some(err.into());
                    } else {
                        let mut response = String::new();
                        match stream.read_to_string(&mut response) {
                            Ok(_)
                                if response.starts_with("HTTP/1.1 200")
                                    || response.starts_with("HTTP/1.0 200") =>
                            {
                                if response.contains(r#""ok":true"#)
                                    || response.contains(r#""ok": true"#)
                                {
                                    return Ok(());
                                }
                                last_error = Some(anyhow::anyhow!(
                                    "healthz response did not report ok=true"
                                ));
                            }
                            Ok(_) => {
                                last_error =
                                    Some(anyhow::anyhow!("healthz returned unexpected response"));
                            }
                            Err(err) => last_error = Some(err.into()),
                        }
                    }
                }
                Err(err) => last_error = Some(err.into()),
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("health check timed out")))
    }
}

fn install_service(
    service: &ServiceSpec,
    command: &HttpCommandSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<()> {
    match service.platform {
        ServicePlatform::Windows => {
            let task_command = windows_task_command(command);
            let args = vec![
                "/Create".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
                "/SC".to_string(),
                "ONLOGON".to_string(),
                "/TR".to_string(),
                task_command,
                "/F".to_string(),
            ];
            require_success(runner.run("schtasks.exe", &args)?, "create scheduled task")?;
        }
        ServicePlatform::Linux => {
            write_text_file(&service.location, &systemd_unit(service, command))?;
            report.push(format!(
                "wrote systemd user unit: {}",
                service.location.display()
            ));
            enable_linger(runner, report);
            require_success(
                runner.run("systemctl", &["--user".into(), "daemon-reload".into()])?,
                "reload systemd user units",
            )?;
            require_success(
                runner.run(
                    "systemctl",
                    &[
                        "--user".into(),
                        "enable".into(),
                        service.platform_id.clone(),
                    ],
                )?,
                "enable systemd user unit",
            )?;
        }
        ServicePlatform::Macos => {
            write_text_file(&service.location, &launch_agent_plist(service, command))?;
            report.push(format!(
                "wrote LaunchAgent plist: {}",
                service.location.display()
            ));
        }
    }
    Ok(())
}

fn uninstall_service(
    service: &ServiceSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<()> {
    match service.platform {
        ServicePlatform::Windows => {
            let args = vec![
                "/Delete".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
                "/F".to_string(),
            ];
            ignore_missing(runner.run("schtasks.exe", &args)?, "delete scheduled task")?;
        }
        ServicePlatform::Linux => {
            let _ = runner.run(
                "systemctl",
                &[
                    "--user".into(),
                    "disable".into(),
                    service.platform_id.clone(),
                ],
            )?;
            if service.location.exists() {
                std::fs::remove_file(&service.location)?;
                report.push(format!(
                    "removed systemd user unit: {}",
                    service.location.display()
                ));
            }
            let _ = runner.run("systemctl", &["--user".into(), "daemon-reload".into()])?;
        }
        ServicePlatform::Macos => {
            if let Some(uid) = current_uid(runner)? {
                let _ = runner.run(
                    "launchctl",
                    &[
                        "bootout".into(),
                        format!("gui/{uid}"),
                        service.location.display().to_string(),
                    ],
                )?;
            }
            if service.location.exists() {
                std::fs::remove_file(&service.location)?;
                report.push(format!(
                    "removed LaunchAgent plist: {}",
                    service.location.display()
                ));
            }
        }
    }
    Ok(())
}

fn start_service(
    service: &ServiceSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<()> {
    match service.platform {
        ServicePlatform::Windows => {
            let args = vec![
                "/Run".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
            ];
            require_success(runner.run("schtasks.exe", &args)?, "start scheduled task")?;
        }
        ServicePlatform::Linux => {
            require_success(
                runner.run(
                    "systemctl",
                    &["--user".into(), "start".into(), service.platform_id.clone()],
                )?,
                "start systemd user unit",
            )?;
        }
        ServicePlatform::Macos => {
            if let Some(uid) = current_uid(runner)? {
                let _ = runner.run(
                    "launchctl",
                    &[
                        "bootout".into(),
                        format!("gui/{uid}"),
                        service.location.display().to_string(),
                    ],
                )?;
                require_success(
                    runner.run(
                        "launchctl",
                        &[
                            "bootstrap".into(),
                            format!("gui/{uid}"),
                            service.location.display().to_string(),
                        ],
                    )?,
                    "bootstrap LaunchAgent",
                )?;
            } else {
                anyhow::bail!("cannot resolve uid for launchctl bootstrap");
            }
        }
    }
    report.push(format!("started HTTP MCP user service: {}", service.name));
    Ok(())
}

fn stop_service_if_running(
    service: &ServiceSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<()> {
    match service.platform {
        ServicePlatform::Windows => {
            let args = vec![
                "/End".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
            ];
            let output = runner.run("schtasks.exe", &args)?;
            if output.success {
                report.push(format!("stopped HTTP MCP user service: {}", service.name));
            } else if !is_not_running_output(&output) && !is_missing_output(&output) {
                require_success(output, "stop scheduled task")?;
            }
        }
        ServicePlatform::Linux => {
            let output = runner.run(
                "systemctl",
                &["--user".into(), "stop".into(), service.platform_id.clone()],
            )?;
            if output.success {
                report.push(format!("stopped HTTP MCP user service: {}", service.name));
            } else if !is_missing_output(&output) {
                require_success(output, "stop systemd user unit")?;
            }
        }
        ServicePlatform::Macos => {
            if let Some(uid) = current_uid(runner)? {
                let output = runner.run(
                    "launchctl",
                    &[
                        "bootout".into(),
                        format!("gui/{uid}"),
                        service.location.display().to_string(),
                    ],
                )?;
                if output.success {
                    report.push(format!("stopped HTTP MCP user service: {}", service.name));
                } else if !is_missing_output(&output) {
                    require_success(output, "bootout LaunchAgent")?;
                }
            }
        }
    }
    Ok(())
}

fn service_exists(service: &ServiceSpec, runner: &dyn CommandRunner) -> anyhow::Result<bool> {
    match service.platform {
        ServicePlatform::Windows => {
            let args = vec![
                "/Query".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
            ];
            let output = runner.run("schtasks.exe", &args)?;
            Ok(output.success)
        }
        ServicePlatform::Linux | ServicePlatform::Macos => Ok(service.location.exists()),
    }
}

fn report_status(
    service: &ServiceSpec,
    runner: &dyn CommandRunner,
    report: &mut McpServiceReport,
) -> anyhow::Result<()> {
    match service.platform {
        ServicePlatform::Windows => {
            let args = vec![
                "/Query".to_string(),
                "/TN".to_string(),
                service.platform_id.clone(),
            ];
            let output = runner.run("schtasks.exe", &args)?;
            if output.success {
                report.push(format!(
                    "HTTP MCP user service is installed: {}",
                    service.name
                ));
                if !output.stdout.is_empty() {
                    report.push(output.stdout);
                }
            } else if is_missing_output(&output) {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
            } else {
                require_success(output, "query scheduled task")?;
            }
        }
        ServicePlatform::Linux => {
            if !service.location.exists() {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
                return Ok(());
            }
            let output = runner.run(
                "systemctl",
                &[
                    "--user".into(),
                    "is-active".into(),
                    service.platform_id.clone(),
                ],
            )?;
            report.push(format!(
                "HTTP MCP user service is installed: {}",
                service.name
            ));
            let active_status = if output.success {
                "yes".to_string()
            } else {
                let status = output.stdout.trim();
                if status.is_empty() {
                    "no".to_string()
                } else {
                    status.to_string()
                }
            };
            report.push(format!("active: {active_status}"));
            report.push(format!("service file: {}", service.location.display()));
        }
        ServicePlatform::Macos => {
            if !service.location.exists() {
                report.push(format!(
                    "HTTP MCP user service is not installed: {}",
                    service.name
                ));
                return Ok(());
            }
            report.push(format!(
                "HTTP MCP user service is installed: {}",
                service.name
            ));
            report.push(format!("service file: {}", service.location.display()));
        }
    }
    Ok(())
}

fn report_health(bind: SocketAddr, health: &dyn HealthChecker, report: &mut McpServiceReport) {
    match health.check(bind, HEALTH_TIMEOUT) {
        Ok(()) => report.push("health check ok: GET /healthz"),
        Err(err) => report.push(format!(
            "health check did not pass within {}s: {err}",
            HEALTH_TIMEOUT.as_secs()
        )),
    }
}

fn require_success(output: CommandOutput, action: &str) -> anyhow::Result<()> {
    if output.success {
        return Ok(());
    }
    anyhow::bail!("{action} failed: {}", command_detail(&output));
}

fn ignore_missing(output: CommandOutput, action: &str) -> anyhow::Result<()> {
    if output.success || is_missing_output(&output) {
        return Ok(());
    }
    anyhow::bail!("{action} failed: {}", command_detail(&output));
}

fn command_detail(output: &CommandOutput) -> String {
    if !output.stderr.is_empty() {
        output.stderr.clone()
    } else if !output.stdout.is_empty() {
        output.stdout.clone()
    } else {
        "command exited unsuccessfully".to_string()
    }
}

fn is_missing_output(output: &CommandOutput) -> bool {
    let detail = command_detail(output).to_ascii_lowercase();
    detail.contains("not found")
        || detail.contains("does not exist")
        || detail.contains("cannot find")
        || detail.contains("could not be found")
        || detail.contains("no such process")
}

fn is_not_running_output(output: &CommandOutput) -> bool {
    let detail = command_detail(output).to_ascii_lowercase();
    detail.contains("not currently running") || detail.contains("not running")
}

fn write_text_file(path: &Path, body: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)?;
    Ok(())
}

fn current_uid(runner: &dyn CommandRunner) -> anyhow::Result<Option<String>> {
    let output = runner.run("id", &["-u".to_string()])?;
    if output.success {
        Ok(Some(output.stdout.trim().to_string()))
    } else {
        Ok(None)
    }
}

fn current_username(runner: &dyn CommandRunner) -> Option<String> {
    if let Ok(output) = runner.run("id", &["-un".to_string()]) {
        if output.success && !output.stdout.trim().is_empty() {
            return Some(output.stdout.trim().to_string());
        }
    }
    std::env::var("USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USERNAME")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}

fn enable_linger(runner: &dyn CommandRunner, report: &mut McpServiceReport) {
    let Some(user) = current_username(runner) else {
        report.push(
            "warning: could not determine current user; run `loginctl enable-linger $USER` manually if the service should survive logout",
        );
        return;
    };
    match runner.run("loginctl", &["enable-linger".to_string(), user.clone()]) {
        Ok(output) if output.success => {
            report.push(format!("enabled systemd linger for user: {user}"));
        }
        Ok(output) => {
            report.push(format!(
                "warning: could not enable systemd linger automatically: {}; run `loginctl enable-linger {user}` manually if needed",
                command_detail(&output)
            ));
        }
        Err(err) => {
            report.push(format!(
                "warning: could not run loginctl enable-linger: {err}; run `loginctl enable-linger {user}` manually if needed"
            ));
        }
    }
}

fn windows_task_command(command: &HttpCommandSpec) -> String {
    let args = command.args();
    if let Some(config) = &command.explicit_config {
        let mut script = format!(
            "$env:GROK_SEARCH_CONFIG={}; & {}",
            powershell_single_quote(&config.display().to_string()),
            powershell_single_quote(&command.exe.display().to_string())
        );
        for arg in args {
            script.push(' ');
            script.push_str(&powershell_single_quote(&arg));
        }
        format!(
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command {}",
            windows_cmd_quote(&script)
        )
    } else {
        let mut out = windows_cmd_quote(&command.exe.display().to_string());
        for arg in args {
            out.push(' ');
            out.push_str(&windows_cmd_quote(&arg));
        }
        out
    }
}

fn systemd_unit(service: &ServiceSpec, command: &HttpCommandSpec) -> String {
    let mut body = format!(
        "[Unit]\nDescription=GrokSearch-rs HTTP MCP ({})\nAfter=network-online.target\n\n[Service]\nType=simple\n",
        service.name
    );
    if let Some(config) = &command.explicit_config {
        body.push_str("Environment=");
        body.push_str(&systemd_quote(&format!(
            "GROK_SEARCH_CONFIG={}",
            config.display()
        )));
        body.push('\n');
    }
    let mut exec = vec![command.exe.display().to_string()];
    exec.extend(command.args());
    body.push_str("ExecStart=");
    body.push_str(
        &exec
            .iter()
            .map(|part| systemd_quote(part))
            .collect::<Vec<_>>()
            .join(" "),
    );
    body.push_str("\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n");
    body
}

fn launch_agent_plist(service: &ServiceSpec, command: &HttpCommandSpec) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
"#,
    );
    body.push_str("  <key>Label</key>\n  <string>");
    body.push_str(&xml_escape(&service.platform_id));
    body.push_str("</string>\n  <key>ProgramArguments</key>\n  <array>\n");
    body.push_str("    <string>");
    body.push_str(&xml_escape(&command.exe.display().to_string()));
    body.push_str("</string>\n");
    for arg in command.args() {
        body.push_str("    <string>");
        body.push_str(&xml_escape(&arg));
        body.push_str("</string>\n");
    }
    body.push_str("  </array>\n");
    if let Some(config) = &command.explicit_config {
        body.push_str(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>GROK_SEARCH_CONFIG</key>\n    <string>",
        );
        body.push_str(&xml_escape(&config.display().to_string()));
        body.push_str("</string>\n  </dict>\n");
    }
    body.push_str(
        r#"  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
"#,
    );
    body
}

fn health_addr(bind: SocketAddr) -> SocketAddr {
    if bind.ip().is_unspecified() {
        match bind.ip() {
            IpAddr::V4(_) => SocketAddr::from(([127, 0, 0, 1], bind.port())),
            IpAddr::V6(_) => SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], bind.port())),
        }
    } else {
        bind
    }
}

fn host_port_for_url(bind: SocketAddr) -> String {
    host_for_authority(bind.ip(), bind.port())
}

fn host_for_http_header(ip: IpAddr, port: u16) -> String {
    host_for_authority(ip, port)
}

fn host_for_authority(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(addr) => format!("{addr}:{port}"),
        IpAddr::V6(addr) => format!("[{addr}]:{port}"),
    }
}

fn windows_cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn systemd_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        outputs: RefCell<Vec<CommandOutput>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outputs: RefCell::new(outputs),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[String]) -> anyhow::Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push((program.to_string(), args.to_vec()));
            Ok(self.outputs.borrow_mut().pop().unwrap_or(CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            }))
        }
    }

    struct FakeHealth {
        ok: bool,
    }

    impl HealthChecker for FakeHealth {
        fn check(&self, _bind: SocketAddr, _timeout: Duration) -> anyhow::Result<()> {
            if self.ok {
                Ok(())
            } else {
                anyhow::bail!("not ready")
            }
        }
    }

    fn ok() -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn context(platform: ServicePlatform, root: &Path) -> ServiceContext {
        let current_exe = root.join("bin dir").join("grok-search-rs.exe");
        std::fs::create_dir_all(current_exe.parent().unwrap()).unwrap();
        std::fs::write(&current_exe, b"current binary").unwrap();
        ServiceContext {
            platform,
            current_exe,
            home_dir: root.join("home"),
            local_app_data: Some(root.join("local app data")),
            explicit_config: Some(root.join("custom config.toml")),
        }
    }

    fn install_options(no_start: bool) -> McpServiceOptions {
        McpServiceOptions {
            name: None,
            command: McpServiceCommand::Install(McpServiceInstallOptions {
                bind: "127.0.0.1:8787".parse().unwrap(),
                path: "/mcp".to_string(),
                allow_origin: Some("http://127.0.0.1:3000".to_string()),
                install_dir: None,
                auth_token_configured: false,
                no_start,
            }),
        }
    }

    #[test]
    fn validates_non_loopback_requires_token() {
        let options = McpServiceInstallOptions {
            bind: "0.0.0.0:8787".parse().unwrap(),
            path: "/mcp".to_string(),
            allow_origin: None,
            install_dir: None,
            auth_token_configured: false,
            no_start: false,
        };

        assert!(options.validate().is_err());
    }

    #[test]
    fn rejects_port_zero_for_service_install() {
        let options = McpServiceInstallOptions {
            bind: "127.0.0.1:0".parse().unwrap(),
            path: "/mcp".to_string(),
            allow_origin: None,
            install_dir: None,
            auth_token_configured: false,
            no_start: false,
        };

        assert!(options.validate().is_err());
    }

    #[test]
    fn renders_windows_task_command_with_forwarded_config() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Windows, temp.path());
        let command = HttpCommandSpec::new(
            ctx.current_exe.clone(),
            "127.0.0.1:8787".parse().unwrap(),
            "/mcp".to_string(),
            Some("http://127.0.0.1:3000".to_string()),
            ctx.explicit_config.clone(),
        );

        let rendered = windows_task_command(&command);

        assert!(rendered.contains("powershell.exe"));
        assert!(rendered.contains("GROK_SEARCH_CONFIG"));
        assert!(rendered.contains("mcp-http"));
        assert!(rendered.contains("--allow-origin"));
    }

    #[test]
    fn renders_systemd_unit() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let service = ServiceSpec::new(DEFAULT_MCP_SERVICE_NAME.to_string(), &ctx).unwrap();
        let command = HttpCommandSpec::new(
            ctx.current_exe.clone(),
            "127.0.0.1:8787".parse().unwrap(),
            "/mcp".to_string(),
            None,
            ctx.explicit_config.clone(),
        );

        let unit = systemd_unit(&service, &command);

        assert!(unit.contains("[Service]"));
        assert!(unit.contains("Environment=\"GROK_SEARCH_CONFIG="));
        assert!(unit.contains("ExecStart="));
        assert!(unit.contains("mcp-http"));
        assert!(unit.contains("--bind"));
    }

    #[test]
    fn renders_launch_agent_plist() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Macos, temp.path());
        let service = ServiceSpec::new(DEFAULT_MCP_SERVICE_NAME.to_string(), &ctx).unwrap();
        let command = HttpCommandSpec::new(
            ctx.current_exe.clone(),
            "127.0.0.1:8787".parse().unwrap(),
            "/mcp".to_string(),
            None,
            ctx.explicit_config.clone(),
        );

        let plist = launch_agent_plist(&service, &command);

        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains(DEFAULT_MACOS_LABEL));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains("GROK_SEARCH_CONFIG"));
    }

    #[test]
    fn install_no_start_does_not_start_or_health_check() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let runner = FakeRunner::default();
        let health = FakeHealth { ok: false };

        let report = run_mcp_service_with(install_options(true), &ctx, &runner, &health).unwrap();

        let calls = runner.calls();
        assert!(calls
            .iter()
            .any(|(_, args)| args.contains(&"enable-linger".into())));
        assert!(calls
            .iter()
            .any(|(_, args)| args.contains(&"enable".into())));
        assert!(calls
            .iter()
            .all(|(_, args)| !args.contains(&"start".into())));
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("--no-start")));
    }

    #[test]
    fn install_starts_and_reports_health() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let runner = FakeRunner::default();
        let health = FakeHealth { ok: true };

        let report = run_mcp_service_with(install_options(false), &ctx, &runner, &health).unwrap();

        let calls = runner.calls();
        assert!(calls.iter().any(|(_, args)| args.contains(&"start".into())));
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("health check ok")));
    }

    #[test]
    fn missing_status_is_reported_without_error() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let runner = FakeRunner::default();
        let health = FakeHealth { ok: true };

        let report = run_mcp_service_with(
            McpServiceOptions {
                name: None,
                command: McpServiceCommand::Status,
            },
            &ctx,
            &runner,
            &health,
        )
        .unwrap();

        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("not installed")));
    }

    #[test]
    fn windows_uninstall_missing_is_reported_without_error() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Windows, temp.path());
        let runner = FakeRunner::with_outputs(vec![CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "ERROR: The system cannot find the file specified.".to_string(),
        }]);
        let health = FakeHealth { ok: true };

        let report = run_mcp_service_with(
            McpServiceOptions {
                name: None,
                command: McpServiceCommand::Uninstall,
            },
            &ctx,
            &runner,
            &health,
        )
        .unwrap();

        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("not installed")));
    }

    #[test]
    fn service_name_is_sanitized_for_file_paths() {
        assert!(service_name(Some("good-name_1.2")).is_ok());
        assert!(service_name(Some("../bad")).is_err());
        assert!(service_name(Some("bad name")).is_err());
    }

    #[test]
    fn copies_current_binary_to_default_install_dir() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let runner = FakeRunner::default();
        let health = FakeHealth { ok: true };

        let report = run_mcp_service_with(install_options(true), &ctx, &runner, &health).unwrap();
        let managed = temp
            .path()
            .join("home")
            .join(".local")
            .join("bin")
            .join("grok-search-rs");

        assert!(managed.exists());
        assert_eq!(std::fs::read(&managed).unwrap(), b"current binary");
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("installed service binary")));
    }

    #[test]
    fn keeps_existing_binary_when_version_is_not_older() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let managed_dir = temp.path().join("managed");
        let managed = managed_dir.join("grok-search-rs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::write(&managed, b"existing binary").unwrap();
        let runner = FakeRunner::with_outputs(vec![
            ok(),
            ok(),
            ok(),
            CommandOutput {
                success: true,
                stdout: format!("grok-search-rs {CURRENT_VERSION}"),
                stderr: String::new(),
            },
        ]);
        let health = FakeHealth { ok: true };
        let mut options = install_options(true);
        if let McpServiceCommand::Install(install) = &mut options.command {
            install.install_dir = Some(managed_dir);
        }

        let report = run_mcp_service_with(options, &ctx, &runner, &health).unwrap();

        assert_eq!(std::fs::read(&managed).unwrap(), b"existing binary");
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("kept existing service binary")));
    }

    #[test]
    fn upgrades_existing_binary_when_current_version_is_higher() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = context(ServicePlatform::Linux, temp.path());
        let managed_dir = temp.path().join("managed");
        let managed = managed_dir.join("grok-search-rs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::write(&managed, b"old binary").unwrap();
        let runner = FakeRunner::with_outputs(vec![
            ok(),
            ok(),
            ok(),
            CommandOutput {
                success: true,
                stdout: "grok-search-rs 0.0.1".to_string(),
                stderr: String::new(),
            },
        ]);
        let health = FakeHealth { ok: true };
        let mut options = install_options(true);
        if let McpServiceCommand::Install(install) = &mut options.command {
            install.install_dir = Some(managed_dir);
        }

        let report = run_mcp_service_with(options, &ctx, &runner, &health).unwrap();

        assert_eq!(std::fs::read(&managed).unwrap(), b"current binary");
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("updated service binary")));
    }

    #[test]
    fn compares_semver_like_versions() {
        assert!(compare_versions("0.3.3", "0.3.2").is_gt());
        assert!(compare_versions("0.3.2", "0.3.2").is_eq());
        assert!(compare_versions("0.3.2", "0.4.0").is_lt());
    }
}
