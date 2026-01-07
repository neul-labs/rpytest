//! Dependency tracking for incremental test invalidation.
//!
//! Maps source files to the tests that import them, so we can
//! selectively re-run only affected tests when source changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tracing::debug;

/// Result of computing affected tests.
#[derive(Debug, Clone)]
pub struct AffectedTests {
    /// Test node IDs that should be re-run.
    pub node_ids: Vec<String>,
    /// Whether the change affects all tests (e.g., conftest.py change).
    pub run_all: bool,
    /// Files that changed.
    pub changed_files: Vec<PathBuf>,
}

/// Graph tracking dependencies between source files and tests.
#[derive(Debug, Default)]
pub struct DependencyGraph {
    /// Map from source file to tests that import it.
    source_to_tests: HashMap<PathBuf, HashSet<String>>,
    /// Map from test file to tests it contains.
    test_file_to_tests: HashMap<PathBuf, HashSet<String>>,
    /// Map from test node ID to its file.
    test_to_file: HashMap<String, PathBuf>,
    /// Conftest files and the directories they affect.
    conftest_scopes: HashMap<PathBuf, PathBuf>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a test and its containing file.
    pub fn add_test(&mut self, node_id: &str, file_path: &Path) {
        let file = file_path.to_path_buf();

        self.test_to_file.insert(node_id.to_string(), file.clone());

        self.test_file_to_tests
            .entry(file)
            .or_default()
            .insert(node_id.to_string());
    }

    /// Register a dependency from a test to a source file.
    pub fn add_dependency(&mut self, node_id: &str, source_file: &Path) {
        self.source_to_tests
            .entry(source_file.to_path_buf())
            .or_default()
            .insert(node_id.to_string());
    }

    /// Register a conftest file and its scope.
    pub fn add_conftest(&mut self, conftest_path: &Path) {
        if let Some(parent) = conftest_path.parent() {
            self.conftest_scopes
                .insert(conftest_path.to_path_buf(), parent.to_path_buf());
        }
    }

    /// Compute which tests are affected by a set of changed files.
    pub fn compute_affected(&self, changed_files: &[PathBuf]) -> AffectedTests {
        let mut affected = HashSet::new();
        let mut run_all = false;

        for path in changed_files {
            // Check if it's a conftest file
            if path
                .file_name()
                .map(|n| n == "conftest.py")
                .unwrap_or(false)
            {
                if let Some(scope) = self.conftest_scopes.get(path) {
                    // All tests under this directory are affected
                    for (test_file, tests) in &self.test_file_to_tests {
                        if test_file.starts_with(scope) {
                            affected.extend(tests.clone());
                        }
                    }

                    // If conftest is at root, run all tests
                    if scope.parent().is_none() || scope == Path::new(".") {
                        run_all = true;
                    }
                } else {
                    // Unknown conftest, run all to be safe
                    run_all = true;
                }
                continue;
            }

            // Check if it's a test file
            let is_test_file = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("test_") || n.ends_with("_test.py"))
                .unwrap_or(false);

            if is_test_file {
                // All tests in this file are affected
                if let Some(tests) = self.test_file_to_tests.get(path) {
                    affected.extend(tests.clone());
                }
            } else {
                // Source file - find all tests that depend on it
                if let Some(tests) = self.source_to_tests.get(path) {
                    affected.extend(tests.clone());
                }
            }
        }

        debug!(
            "Computed {} affected tests from {} changed files (run_all={})",
            affected.len(),
            changed_files.len(),
            run_all
        );

        AffectedTests {
            node_ids: affected.into_iter().collect(),
            run_all,
            changed_files: changed_files.to_vec(),
        }
    }

    /// Get all registered tests.
    pub fn all_tests(&self) -> Vec<String> {
        self.test_to_file.keys().cloned().collect()
    }

    /// Get the file containing a test.
    pub fn test_file(&self, node_id: &str) -> Option<&Path> {
        self.test_to_file.get(node_id).map(|p| p.as_path())
    }

    /// Clear the graph.
    pub fn clear(&mut self) {
        self.source_to_tests.clear();
        self.test_file_to_tests.clear();
        self.test_to_file.clear();
        self.conftest_scopes.clear();
    }

    /// Get statistics about the graph.
    pub fn stats(&self) -> DependencyStats {
        DependencyStats {
            test_count: self.test_to_file.len(),
            test_file_count: self.test_file_to_tests.len(),
            source_file_count: self.source_to_tests.len(),
            conftest_count: self.conftest_scopes.len(),
        }
    }
}

/// Statistics about the dependency graph.
#[derive(Debug, Clone)]
pub struct DependencyStats {
    /// Number of tests tracked.
    pub test_count: usize,
    /// Number of test files.
    pub test_file_count: usize,
    /// Number of source files with known dependents.
    pub source_file_count: usize,
    /// Number of conftest files.
    pub conftest_count: usize,
}

/// Build a basic dependency graph from test inventory.
///
/// This creates a simple mapping from test files to tests.
/// Full import dependency tracking requires Python AST analysis.
pub fn build_from_inventory(tests: &[(String, String)]) -> DependencyGraph {
    let mut graph = DependencyGraph::new();

    for (node_id, file_path) in tests {
        let path = PathBuf::from(file_path);
        graph.add_test(node_id, &path);

        // If there's a conftest in the same directory, register it
        if let Some(parent) = path.parent() {
            let conftest = parent.join("conftest.py");
            if conftest.exists() {
                graph.add_conftest(&conftest);
            }
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_test() {
        let mut graph = DependencyGraph::new();
        graph.add_test("test_foo.py::test_bar", Path::new("test_foo.py"));

        assert_eq!(graph.test_to_file.len(), 1);
        assert_eq!(
            graph.test_file("test_foo.py::test_bar"),
            Some(Path::new("test_foo.py"))
        );
    }

    #[test]
    fn test_compute_affected_test_file() {
        let mut graph = DependencyGraph::new();
        graph.add_test("test_foo.py::test_one", Path::new("test_foo.py"));
        graph.add_test("test_foo.py::test_two", Path::new("test_foo.py"));
        graph.add_test("test_bar.py::test_three", Path::new("test_bar.py"));

        let affected = graph.compute_affected(&[PathBuf::from("test_foo.py")]);

        assert_eq!(affected.node_ids.len(), 2);
        assert!(!affected.run_all);
    }

    #[test]
    fn test_compute_affected_source_file() {
        let mut graph = DependencyGraph::new();
        graph.add_test("test_foo.py::test_one", Path::new("test_foo.py"));
        graph.add_test("test_bar.py::test_two", Path::new("test_bar.py"));

        // Add dependency: test_one imports utils.py
        graph.add_dependency("test_foo.py::test_one", Path::new("utils.py"));

        let affected = graph.compute_affected(&[PathBuf::from("utils.py")]);

        assert_eq!(affected.node_ids.len(), 1);
        assert_eq!(affected.node_ids[0], "test_foo.py::test_one");
    }

    #[test]
    fn test_compute_affected_conftest() {
        let mut graph = DependencyGraph::new();
        graph.add_test(
            "tests/test_foo.py::test_one",
            Path::new("tests/test_foo.py"),
        );
        graph.add_test(
            "tests/test_bar.py::test_two",
            Path::new("tests/test_bar.py"),
        );
        graph.add_test(
            "other/test_baz.py::test_three",
            Path::new("other/test_baz.py"),
        );

        graph.add_conftest(Path::new("tests/conftest.py"));

        let affected = graph.compute_affected(&[PathBuf::from("tests/conftest.py")]);

        // Both tests under tests/ should be affected
        assert_eq!(affected.node_ids.len(), 2);
        assert!(!affected.run_all);
    }

    #[test]
    fn test_all_tests() {
        let mut graph = DependencyGraph::new();
        graph.add_test("test_a.py::test_1", Path::new("test_a.py"));
        graph.add_test("test_b.py::test_2", Path::new("test_b.py"));

        let all = graph.all_tests();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_stats() {
        let mut graph = DependencyGraph::new();
        graph.add_test("test_a.py::test_1", Path::new("test_a.py"));
        graph.add_test("test_a.py::test_2", Path::new("test_a.py"));
        graph.add_dependency("test_a.py::test_1", Path::new("src/utils.py"));
        graph.add_conftest(Path::new("conftest.py"));

        let stats = graph.stats();
        assert_eq!(stats.test_count, 2);
        assert_eq!(stats.test_file_count, 1);
        assert_eq!(stats.source_file_count, 1);
        assert_eq!(stats.conftest_count, 1);
    }
}
