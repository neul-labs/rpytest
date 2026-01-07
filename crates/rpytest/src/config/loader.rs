//! Configuration file parsing with pytest precedence rules.
//!
//! Precedence (highest to lowest):
//! 1. pytest.ini
//! 2. pyproject.toml [tool.pytest.ini_options]
//! 3. tox.ini [pytest]
//! 4. setup.cfg [tool:pytest]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use configparser::ini::Ini;
use thiserror::Error;

/// Errors that can occur during configuration loading.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file {0}: {1}")]
    ReadError(PathBuf, std::io::Error),

    #[error("Failed to parse config file {0}: {1}")]
    ParseError(PathBuf, String),

    #[error("Invalid configuration: {0}")]
    Invalid(String),
}

/// Parsed pytest configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Additional command-line arguments to always pass.
    pub addopts: Vec<String>,

    /// Paths to search for tests.
    pub testpaths: Vec<PathBuf>,

    /// Registered markers.
    pub markers: Vec<String>,

    /// Patterns for test file names.
    pub python_files: Vec<String>,

    /// Patterns for test class names.
    pub python_classes: Vec<String>,

    /// Patterns for test function names.
    pub python_functions: Vec<String>,

    /// Minimum pytest version required.
    pub minversion: Option<String>,

    /// Filter warnings.
    pub filterwarnings: Vec<String>,

    /// Directories to not recurse into.
    pub norecursedirs: Vec<String>,

    /// Source of this configuration.
    pub source: Option<PathBuf>,

    /// Additional options from config file.
    pub extra: HashMap<String, String>,
}

impl Config {
    /// Create a new empty configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge another configuration into this one.
    /// Values from `other` override values in `self`.
    pub fn merge(&mut self, other: Config) {
        if !other.addopts.is_empty() {
            self.addopts = other.addopts;
        }
        if !other.testpaths.is_empty() {
            self.testpaths = other.testpaths;
        }
        if !other.markers.is_empty() {
            self.markers = other.markers;
        }
        if !other.python_files.is_empty() {
            self.python_files = other.python_files;
        }
        if !other.python_classes.is_empty() {
            self.python_classes = other.python_classes;
        }
        if !other.python_functions.is_empty() {
            self.python_functions = other.python_functions;
        }
        if other.minversion.is_some() {
            self.minversion = other.minversion;
        }
        if !other.filterwarnings.is_empty() {
            self.filterwarnings = other.filterwarnings;
        }
        if !other.norecursedirs.is_empty() {
            self.norecursedirs = other.norecursedirs;
        }
        if other.source.is_some() {
            self.source = other.source;
        }
        for (k, v) in other.extra {
            self.extra.insert(k, v);
        }
    }
}

/// Load configuration from the given root directory.
///
/// Searches for configuration files in precedence order and merges them.
pub fn load_config(rootdir: &Path) -> Result<Config, ConfigError> {
    let mut config = Config::new();

    // Try each config file in reverse precedence order (lowest first)
    // so higher precedence files override

    // 4. setup.cfg
    let setup_cfg = rootdir.join("setup.cfg");
    if setup_cfg.exists() {
        if let Ok(cfg) = load_ini_config(&setup_cfg, "tool:pytest") {
            config.merge(cfg);
        }
    }

    // 3. tox.ini
    let tox_ini = rootdir.join("tox.ini");
    if tox_ini.exists() {
        if let Ok(cfg) = load_ini_config(&tox_ini, "pytest") {
            config.merge(cfg);
        }
    }

    // 2. pyproject.toml
    let pyproject = rootdir.join("pyproject.toml");
    if pyproject.exists() {
        if let Ok(cfg) = load_pyproject_config(&pyproject) {
            config.merge(cfg);
        }
    }

    // 1. pytest.ini (highest precedence)
    let pytest_ini = rootdir.join("pytest.ini");
    if pytest_ini.exists() {
        if let Ok(cfg) = load_ini_config(&pytest_ini, "pytest") {
            config.merge(cfg);
        }
    }

    Ok(config)
}

/// Load configuration from an INI file.
fn load_ini_config(path: &Path, section: &str) -> Result<Config, ConfigError> {
    let mut ini = Ini::new();
    ini.load(path)
        .map_err(|e| ConfigError::ParseError(path.to_path_buf(), e))?;

    let mut config = Config::new();
    config.source = Some(path.to_path_buf());

    // Get the section map
    if let Some(section_map) = ini.get_map_ref().get(section) {
        for (key, value) in section_map {
            if let Some(val) = value {
                apply_config_value(&mut config, key, val);
            }
        }
    }

    Ok(config)
}

/// Load configuration from pyproject.toml.
fn load_pyproject_config(path: &Path) -> Result<Config, ConfigError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| ConfigError::ReadError(path.to_path_buf(), e))?;

    let toml_value: toml::Value = content
        .parse()
        .map_err(|e: toml::de::Error| ConfigError::ParseError(path.to_path_buf(), e.to_string()))?;

    let mut config = Config::new();
    config.source = Some(path.to_path_buf());

    // Navigate to [tool.pytest.ini_options]
    if let Some(tool) = toml_value.get("tool") {
        if let Some(pytest) = tool.get("pytest") {
            if let Some(ini_options) = pytest.get("ini_options") {
                if let Some(table) = ini_options.as_table() {
                    for (key, value) in table {
                        let val_str = match value {
                            toml::Value::String(s) => s.clone(),
                            toml::Value::Array(arr) => arr
                                .iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                            toml::Value::Integer(i) => i.to_string(),
                            toml::Value::Boolean(b) => b.to_string(),
                            _ => continue,
                        };
                        apply_config_value(&mut config, key, &val_str);
                    }
                }
            }
        }
    }

    Ok(config)
}

/// Apply a single configuration key-value pair to the config struct.
fn apply_config_value(config: &mut Config, key: &str, value: &str) {
    match key {
        "addopts" => {
            config.addopts = shell_words::split(value)
                .unwrap_or_else(|_| value.split_whitespace().map(String::from).collect());
        }
        "testpaths" => {
            config.testpaths = value
                .lines()
                .flat_map(|line| line.split_whitespace())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect();
        }
        "markers" => {
            config.markers = value
                .lines()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect();
        }
        "python_files" => {
            config.python_files = value.split_whitespace().map(String::from).collect();
        }
        "python_classes" => {
            config.python_classes = value.split_whitespace().map(String::from).collect();
        }
        "python_functions" => {
            config.python_functions = value.split_whitespace().map(String::from).collect();
        }
        "minversion" => {
            config.minversion = Some(value.trim().to_string());
        }
        "filterwarnings" => {
            config.filterwarnings = value
                .lines()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect();
        }
        "norecursedirs" => {
            config.norecursedirs = value.split_whitespace().map(String::from).collect();
        }
        _ => {
            config.extra.insert(key.to_string(), value.to_string());
        }
    }
}

// Simple shell word splitting (handles quotes)
mod shell_words {
    pub fn split(s: &str) -> Result<Vec<String>, ()> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escape_next = false;

        for c in s.chars() {
            if escape_next {
                current.push(c);
                escape_next = false;
                continue;
            }

            match c {
                '\\' if !in_single_quote => {
                    escape_next = true;
                }
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                }
                ' ' | '\t' | '\n' if !in_single_quote && !in_double_quote => {
                    if !current.is_empty() {
                        words.push(current);
                        current = String::new();
                    }
                }
                _ => {
                    current.push(c);
                }
            }
        }

        if in_single_quote || in_double_quote {
            return Err(());
        }

        if !current.is_empty() {
            words.push(current);
        }

        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn load_empty_config() {
        let temp = TempDir::new().unwrap();
        let config = load_config(temp.path()).unwrap();
        assert!(config.addopts.is_empty());
        assert!(config.testpaths.is_empty());
    }

    #[test]
    fn load_pytest_ini() {
        let temp = TempDir::new().unwrap();
        let ini_path = temp.path().join("pytest.ini");
        fs::write(
            &ini_path,
            r#"[pytest]
addopts = -v --tb=short
testpaths = tests
python_files = test_*.py
"#,
        )
        .unwrap();

        let config = load_config(temp.path()).unwrap();
        assert_eq!(config.addopts, vec!["-v", "--tb=short"]);
        assert_eq!(config.testpaths, vec![PathBuf::from("tests")]);
        assert_eq!(config.python_files, vec!["test_*.py"]);
    }

    #[test]
    fn load_pyproject_toml() {
        let temp = TempDir::new().unwrap();
        let toml_path = temp.path().join("pyproject.toml");
        fs::write(
            &toml_path,
            r#"[tool.pytest.ini_options]
addopts = "-v --tb=short"
testpaths = ["tests", "integration"]
minversion = "7.0"
"#,
        )
        .unwrap();

        let config = load_config(temp.path()).unwrap();
        assert_eq!(config.addopts, vec!["-v", "--tb=short"]);
        assert_eq!(
            config.testpaths,
            vec![PathBuf::from("tests"), PathBuf::from("integration")]
        );
        assert_eq!(config.minversion, Some("7.0".to_string()));
    }

    #[test]
    fn pytest_ini_overrides_pyproject() {
        let temp = TempDir::new().unwrap();

        // Lower precedence
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"[tool.pytest.ini_options]
addopts = "-q"
testpaths = ["from_pyproject"]
"#,
        )
        .unwrap();

        // Higher precedence
        fs::write(
            temp.path().join("pytest.ini"),
            r#"[pytest]
addopts = -v
"#,
        )
        .unwrap();

        let config = load_config(temp.path()).unwrap();
        // addopts from pytest.ini wins
        assert_eq!(config.addopts, vec!["-v"]);
        // testpaths not in pytest.ini, so pyproject.toml value is kept
        assert_eq!(config.testpaths, vec![PathBuf::from("from_pyproject")]);
    }

    #[test]
    fn shell_words_split() {
        assert_eq!(
            shell_words::split("-v --tb=short").unwrap(),
            vec!["-v", "--tb=short"]
        );

        assert_eq!(
            shell_words::split(r#"-k "test and not slow""#).unwrap(),
            vec!["-k", "test and not slow"]
        );

        assert_eq!(
            shell_words::split("-k 'foo bar'").unwrap(),
            vec!["-k", "foo bar"]
        );
    }
}
