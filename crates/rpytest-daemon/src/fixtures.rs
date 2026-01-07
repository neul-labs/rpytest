//! Session fixture reuse management.

use crate::models::{FixtureConfig, FixtureScope, FixtureState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing::debug;

/// Session state for a pytest session with warm fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub repo_path: String,
    pub python_path: String,
    pub fixtures: HashMap<String, FixtureState>,
    pub created_at: f64,
    pub last_run_at: f64,
    pub total_runs: u32,
    pub enabled: bool,
}

impl Default for SessionState {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        SessionState {
            session_id: String::new(),
            repo_path: String::new(),
            python_path: String::new(),
            fixtures: HashMap::new(),
            created_at: now,
            last_run_at: now,
            total_runs: 0,
            enabled: false,
        }
    }
}

/// Manages session fixture reuse across test runs.
#[derive(Debug, Clone)]
pub struct FixtureManager {
    /// Sessions by context_id
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    /// Default configuration
    config: FixtureConfig,
    /// Maximum fixture age in seconds
    max_age_seconds: f64,
}

impl Default for FixtureManager {
    fn default() -> Self {
        FixtureManager {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            config: FixtureConfig::default(),
            max_age_seconds: 600.0, // 10 minutes
        }
    }
}

impl FixtureManager {
    /// Create a new fixture manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new session state for a context.
    pub fn create_session(
        &self,
        context_id: &str,
        repo_path: &PathBuf,
        python_path: &PathBuf,
    ) -> SessionState {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let session = SessionState {
            session_id: format!("{}-{}", context_id, now as u64),
            repo_path: repo_path.to_string_lossy().to_string(),
            python_path: python_path.to_string_lossy().to_string(),
            fixtures: HashMap::new(),
            created_at: now,
            last_run_at: now,
            total_runs: 0,
            enabled: false,
        };

        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(context_id.to_string(), session.clone());

        debug!("Created fixture session for {}", context_id);
        session
    }

    /// Get session state for a context.
    pub fn get_session(&self, context_id: &str) -> Option<SessionState> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(context_id).cloned()
    }

    /// Enable fixture reuse for a context.
    pub fn enable_reuse(&self, context_id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(context_id) {
            session.enabled = true;
            debug!("Enabled fixture reuse for {}", context_id);
            true
        } else {
            false
        }
    }

    /// Disable fixture reuse and teardown all fixtures.
    pub fn disable_reuse(&self, context_id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(context_id) {
            session.enabled = false;
            session.fixtures.clear();
            debug!("Disabled fixture reuse for {}", context_id);
            true
        } else {
            false
        }
    }

    /// Mark a fixture as used in the current run.
    pub fn mark_fixture_used(&self, context_id: &str, name: &str, scope: FixtureScope) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(context_id) {
            if !session.enabled {
                return;
            }

            let fixture =
                session
                    .fixtures
                    .entry(name.to_string())
                    .or_insert_with(|| FixtureState {
                        name: name.to_string(),
                        scope,
                        created_at: now,
                        last_used: now,
                        use_count: 0,
                        teardown_pending: false,
                    });
            fixture.last_used = now;
            fixture.use_count += 1;
        }
    }

    /// Mark a test run as complete.
    pub fn mark_run_complete(&self, context_id: &str) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(context_id) {
            session.last_run_at = now;
            session.total_runs += 1;
        }
    }

    /// Get fixtures that haven't been used recently.
    pub fn get_stale_fixtures(&self, context_id: &str) -> Vec<String> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let mut stale = Vec::new();

        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(context_id) {
            for (name, state) in &session.fixtures {
                if now - state.last_used > self.max_age_seconds {
                    stale.push(name.clone());
                }
            }
        }

        stale
    }

    /// Clear all sessions.
    pub fn clear_all(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.clear();
    }

    /// Remove a specific context's session.
    pub fn remove_session(&self, context_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(context_id);
    }

    /// Get session count.
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Configure fixture reuse settings.
    pub fn configure(&mut self, config: FixtureConfig) {
        self.config = config.clone();
        self.max_age_seconds = config.max_age_seconds;
    }

    /// Get current configuration.
    pub fn get_config(&self) -> &FixtureConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_create_session() {
        let manager = FixtureManager::new();
        let repo_path = PathBuf::from("/test/repo");
        let python_path = PathBuf::from("/usr/bin/python");

        let session = manager.create_session("ctx-1", &repo_path, &python_path);

        assert!(!session.session_id.is_empty());
        assert_eq!(session.repo_path, "/test/repo");
        assert!(manager.session_count() == 1);
    }

    #[test]
    fn test_enable_disable() {
        let manager = FixtureManager::new();
        let repo_path = PathBuf::from("/test/repo");
        let python_path = PathBuf::from("/usr/bin/python");

        manager.create_session("ctx-1", &repo_path, &python_path);
        assert!(manager.enable_reuse("ctx-1"));
        assert!(manager.disable_reuse("ctx-1"));
    }

    #[test]
    fn test_mark_fixture_used() {
        let manager = FixtureManager::new();
        let repo_path = PathBuf::from("/test/repo");
        let python_path = PathBuf::from("/usr/bin/python");

        manager.create_session("ctx-1", &repo_path, &python_path);
        manager.enable_reuse("ctx-1");
        manager.mark_fixture_used("ctx-1", "db_connection", FixtureScope::Session);

        let session = manager.get_session("ctx-1").unwrap();
        assert!(session.fixtures.contains_key("db_connection"));
        assert_eq!(session.fixtures["db_connection"].use_count, 1);
    }
}
