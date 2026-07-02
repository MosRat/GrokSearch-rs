use tracing_subscriber::EnvFilter;

use crate::args::{Cli, Command};

pub(crate) const EFFECTIVE_FILTER_ENV: &str = "GROK_SEARCH_LOG_EFFECTIVE_FILTER";
pub(crate) const FILTER_SOURCE_ENV: &str = "GROK_SEARCH_LOG_FILTER_SOURCE";
pub(crate) const EXPLICIT_ENV: &str = "GROK_SEARCH_LOG_EXPLICIT";

const GROK_SEARCH_LOG_ENV: &str = "GROK_SEARCH_LOG";
const RUST_LOG_ENV: &str = "RUST_LOG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogSettings {
    pub(crate) filter: String,
    pub(crate) source: &'static str,
    pub(crate) explicit: bool,
}

pub(crate) fn init(cli: &Cli) {
    let mut settings = resolve_from_env(cli);
    let filter = match EnvFilter::try_new(&settings.filter) {
        Ok(filter) => filter,
        Err(_) => {
            settings = LogSettings {
                filter: default_filter(cli).to_string(),
                source: "default",
                explicit: false,
            };
            EnvFilter::new(&settings.filter)
        }
    };
    std::env::set_var(EFFECTIVE_FILTER_ENV, &settings.filter);
    std::env::set_var(FILTER_SOURCE_ENV, settings.source);
    std::env::set_var(EXPLICIT_ENV, settings.explicit.to_string());

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
}

fn resolve_from_env(cli: &Cli) -> LogSettings {
    resolve(cli, std::env::vars())
}

pub(crate) fn resolve<I, K, V>(cli: &Cli, vars: I) -> LogSettings
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let mut rust_log = None;
    for (key, value) in vars {
        let key = key.as_ref();
        let value = value.into();
        if value.trim().is_empty() {
            continue;
        }
        match key {
            GROK_SEARCH_LOG_ENV => {
                return LogSettings {
                    filter: value,
                    source: GROK_SEARCH_LOG_ENV,
                    explicit: true,
                };
            }
            RUST_LOG_ENV => rust_log = Some(value),
            _ => {}
        }
    }

    if let Some(filter) = rust_log {
        return LogSettings {
            filter,
            source: RUST_LOG_ENV,
            explicit: true,
        };
    }

    LogSettings {
        filter: default_filter(cli).to_string(),
        source: "default",
        explicit: false,
    }
}

fn default_filter(cli: &Cli) -> &'static str {
    if cli.init_alias {
        return "warn";
    }
    match cli.command.as_ref() {
        None | Some(Command::Mcp) => "off",
        Some(Command::McpHttp(_)) => "info",
        Some(_) => "warn",
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn mcp_stdio_defaults_to_off() {
        let settings = resolve(&cli(&["grok-search-rs", "mcp"]), [] as [(&str, &str); 0]);
        assert_eq!(settings.filter, "off");
        assert_eq!(settings.source, "default");
        assert!(!settings.explicit);
    }

    #[test]
    fn mcp_http_defaults_to_info() {
        let settings = resolve(
            &cli(&["grok-search-rs", "mcp-http"]),
            [] as [(&str, &str); 0],
        );
        assert_eq!(settings.filter, "info");
        assert_eq!(settings.source, "default");
        assert!(!settings.explicit);
    }

    #[test]
    fn regular_cli_defaults_to_warn() {
        let settings = resolve(&cli(&["grok-search-rs", "doctor"]), [] as [(&str, &str); 0]);
        assert_eq!(settings.filter, "warn");
        assert_eq!(settings.source, "default");
        assert!(!settings.explicit);
    }

    #[test]
    fn grok_search_log_overrides_rust_log() {
        let settings = resolve(
            &cli(&["grok-search-rs", "doctor"]),
            [
                ("RUST_LOG", "error"),
                ("GROK_SEARCH_LOG", "grok_search=debug"),
            ],
        );
        assert_eq!(settings.filter, "grok_search=debug");
        assert_eq!(settings.source, "GROK_SEARCH_LOG");
        assert!(settings.explicit);
    }

    #[test]
    fn rust_log_is_used_as_fallback() {
        let settings = resolve(
            &cli(&["grok-search-rs", "doctor"]),
            [("RUST_LOG", "grok_search=trace")],
        );
        assert_eq!(settings.filter, "grok_search=trace");
        assert_eq!(settings.source, "RUST_LOG");
        assert!(settings.explicit);
    }
}
