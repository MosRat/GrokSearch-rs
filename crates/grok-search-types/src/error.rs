use thiserror::Error;

#[derive(Debug, Error)]
pub enum GrokSearchError {
    #[error("missing required config: {0}")]
    MissingConfig(&'static str),
    #[error("config error: {0}")]
    Config(String),
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("upstream timeout: {0}")]
    Timeout(String),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("security policy rejected request: {0}")]
    SecurityPolicy(String),
    #[error("oauth error: {0}")]
    OAuth(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(String),
}

impl GrokSearchError {
    /// JSON-RPC 2.0 error code mapping. See https://www.jsonrpc.org/specification#error_object
    pub fn code(&self) -> i32 {
        match self {
            // -32700 Parse error: invalid JSON
            GrokSearchError::Parse(_) => -32700,
            // -32602 Invalid params
            GrokSearchError::InvalidParams(_) => -32602,
            GrokSearchError::SecurityPolicy(_) => -32602,
            // -32004 (server-defined) resource not found
            GrokSearchError::NotFound(_) => -32004,
            // -32002 (server-defined) upstream timeout
            GrokSearchError::Timeout(_) => -32002,
            // -32001 (server-defined) upstream / provider failure
            GrokSearchError::Provider(_) => -32001,
            GrokSearchError::Upstream(_) => -32001,
            // -32005 (server-defined) OAuth setup / refresh failure
            GrokSearchError::OAuth(_) => -32005,
            // -32003 (server-defined) missing config
            GrokSearchError::MissingConfig(_) => -32003,
            GrokSearchError::Config(_) => -32003,
            // -32006 (server-defined) local IO failure
            GrokSearchError::Io(_) => -32006,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            GrokSearchError::MissingConfig(_) => "missing_config",
            GrokSearchError::Config(_) => "config",
            GrokSearchError::InvalidParams(_) => "invalid_params",
            GrokSearchError::NotFound(_) => "not_found",
            GrokSearchError::Timeout(_) => "timeout",
            GrokSearchError::Upstream(_) => "upstream",
            GrokSearchError::Provider(_) => "provider",
            GrokSearchError::SecurityPolicy(_) => "security_policy",
            GrokSearchError::OAuth(_) => "oauth",
            GrokSearchError::Parse(_) => "parse",
            GrokSearchError::Io(_) => "io",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            GrokSearchError::Timeout(_)
                | GrokSearchError::Upstream(_)
                | GrokSearchError::Provider(_)
        )
    }

    pub fn hint(&self) -> &'static str {
        match self {
            GrokSearchError::MissingConfig(_) => {
                "Set the required environment variable or TOML config key."
            }
            GrokSearchError::Config(_) => "Check GROK_SEARCH_CONFIG and the documented TOML keys.",
            GrokSearchError::InvalidParams(_) => "Validate the tool arguments against the schema.",
            GrokSearchError::NotFound(_) => {
                "Check the identifier or session id and retry after a fresh search if needed."
            }
            GrokSearchError::Timeout(_) => "Retry later or increase GROK_SEARCH_TIMEOUT_SECONDS.",
            GrokSearchError::Upstream(_) | GrokSearchError::Provider(_) => {
                "Check provider credentials, upstream status, proxy settings, and response limits."
            }
            GrokSearchError::SecurityPolicy(_) => {
                "Use a public http/https URL; private and localhost targets are blocked."
            }
            GrokSearchError::OAuth(_) => "Run login/status diagnostics or refresh the OAuth token.",
            GrokSearchError::Parse(_) => "The upstream response or local payload was malformed.",
            GrokSearchError::Io(_) => "Check filesystem permissions and paths.",
        }
    }

    pub fn context(&self) -> serde_json::Value {
        let message = self.to_string();
        serde_json::json!({
            "message": message,
        })
    }

    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code(),
            "kind": self.kind(),
            "retryable": self.retryable(),
            "hint": self.hint(),
            "context": self.context(),
        })
    }
}

pub type Result<T> = std::result::Result<T, GrokSearchError>;
