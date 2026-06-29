mod diagnostics;
pub mod docs;
mod loader;
mod model;
mod paths;
mod reader;
mod schema;
mod template;
mod util;

pub const MAX_INLINE_SOURCES_LIMIT: usize = 20;

pub use loader::{config_template, write_template, ConfigLoadError, InitOutcome, CONFIG_TEMPLATE};
pub use model::{AuthMode, Config, Transport};
pub use paths::{auth_path, auth_path_for, config_path, config_path_for};
pub use util::normalize_v1_base;
