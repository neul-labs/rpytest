//! Configuration file loading for pytest compatibility.

mod loader;

pub use loader::{load_config, Config, ConfigError};
