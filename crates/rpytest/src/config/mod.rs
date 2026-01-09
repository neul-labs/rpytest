//! Configuration file loading for pytest compatibility.

mod loader;

#[allow(unused_imports)]
pub use loader::{load_config, Config, ConfigError};
