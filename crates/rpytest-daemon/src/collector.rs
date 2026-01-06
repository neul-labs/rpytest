//! Native AST-based test collection using RustPython parser.

use crate::error::{Result, DaemonError};
use crate::models::NativeTestNode;
use rayon::prelude::*;
use rustpython_parser::parse_program;
use rustpython_ast::{self as ast, ExprAttribute, ExprCall, ExprName};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, warn};

/// Native test collector using RustPython AST parsing.
///
/// This is faster than pytest collection because it:
/// 1. Doesn't import pytest or test modules
/// 2. Uses pure Rust AST parsing (rustpython-parser)
/// 3. Doesn't run conftest.py files
/// 4. Can cache results to disk
#[derive(Debug, Clone)]
pub struct NativeCollector {
    /// Root path of the repository
    repo_path: PathBuf,
    /// Built-in pytest fixtures that require pytest
    builtin_fixtures: Vec<&'static str>,
    /// Collected tests (shared for thread safety)
    tests: Arc<Mutex<Vec<NativeTestNode>>>,
}

impl Default for NativeCollector {
    fn default() -> Self {
        NativeCollector {
            repo_path: PathBuf::from("."),
            builtin_fixtures: vec![
                "capsys", "capfd", "capsysbinary", "capfdbinary", "caplog",
                "tmp_path", "tmp_path_factory", "tmpdir", "tmpdir_factory",
                "request", "pytestconfig", "cache", "recwarn",
                "monkeypatch", "doctest_namespace",
            ],
            tests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl NativeCollector {
    /// Create a new collector for the given repository.
    pub fn new(repo_path: &Path) -> Self {
        NativeCollector {
            repo_path: repo_path.to_path_buf(),
            builtin_fixtures: vec![
                "capsys", "capfd", "capsysbinary", "capfdbinary", "caplog",
                "tmp_path", "tmp_path_factory", "tmpdir", "tmpdir_factory",
                "request", "pytestconfig", "cache", "recwarn",
                "monkeypatch", "doctest_namespace",
            ],
            tests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Collect all tests using AST parsing.
    pub fn collect(&self) -> Result<Vec<NativeTestNode>> {
        self.tests.lock().unwrap().clear();

        // Find all test files
        let test_files = self.find_test_files()?;

        // Parse files in parallel
        test_files.par_iter().for_each(|file| {
            if let Ok(nodes) = self.parse_test_file(file) {
                let mut tests = self.tests.lock().unwrap();
                tests.extend(nodes);
            }
        });

        let tests = self.tests.lock().unwrap().clone();
        debug!("Collected {} native tests", tests.len());
        Ok(tests)
    }

    /// Find all test files in the repository.
    fn find_test_files(&self) -> Result<Vec<PathBuf>> {
        let mut test_files = Vec::new();

        // Common test file patterns
        for entry in walkdir::WalkDir::new(&self.repo_path)
            .follow_links(true)
            .into_iter()
        {
            let entry = entry?;
            let path = entry.path().to_path_buf();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Skip venv and hidden directories
            let path_str = path.to_string_lossy();
            if path_str.contains("/.venv/") || path_str.contains("/venv/")
                || path_str.contains("/node_modules/")
                || path_str.contains("/.git/")
            {
                continue;
            }

            // Check if it's a Python test file
            if let Some(ext) = path.extension() {
                if ext == "py" {
                    let file_name = path.file_name().unwrap().to_string_lossy();
                    // Match test_*.py or *_test.py patterns
                    if file_name.starts_with("test_")
                        || file_name.ends_with("_test.py")
                    {
                        test_files.push(path);
                    }
                }
            }
        }

        debug!("Found {} test files", test_files.len());
        Ok(test_files)
    }

    /// Parse a single test file and extract test nodes.
    fn parse_test_file(&self, path: &PathBuf) -> Result<Vec<NativeTestNode>> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", path.display(), e);
                return Ok(Vec::new());
            }
        };

        // Parse using rustpython-parser
        let syntax = parse_program(&content, "<test>");

        match syntax {
            Ok(stmts) => self.extract_tests_from_ast(&stmts, path),
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
                Ok(Vec::new())
            }
        }
    }

    /// Extract test nodes from Python AST statements.
    fn extract_tests_from_ast(
        &self,
        stmts: &Vec<ast::Stmt>,
        path: &PathBuf,
    ) -> Result<Vec<NativeTestNode>> {
        let mut tests = Vec::new();
        let file_path = path.to_string_lossy().to_string();

        // Get relative path from repo root
        let relative_path = if let Ok(stripped) = path.strip_prefix(&self.repo_path) {
            stripped.to_string_lossy().to_string()
        } else {
            file_path.clone()
        };

        // Parse conftest fixtures if this is a conftest
        let mut conftest_fixtures: Vec<String> = Vec::new();
        let is_conftest = path.file_name().unwrap().to_string_lossy() == "conftest.py";

        if is_conftest {
            conftest_fixtures = self.extract_conftest_fixtures(stmts);
        }

        // Extract tests from statements
        self.extract_stmt_items(
            stmts,
            "",
            &relative_path,
            &conftest_fixtures,
            &mut tests,
        );

        Ok(tests)
    }

    /// Extract fixtures from conftest.py
    fn extract_conftest_fixtures(&self, stmts: &Vec<ast::Stmt>) -> Vec<String> {
        let mut fixtures = Vec::new();

        for stmt in stmts {
            if let ast::Stmt::FunctionDef(func) = stmt {
                // Check for @pytest.fixture decorator
                for decorator in &func.decorator_list {
                    if let Some(name) = self.get_decorator_name(decorator) {
                        if name == "fixture" {
                            fixtures.push(func.name.to_string());
                        }
                    }
                }
            }
        }

        fixtures
    }

    /// Get decorator name from a decorator expression.
    fn get_decorator_name(&self, decorator: &ast::Expr) -> Option<String> {
        match decorator {
            // Handle pytest.mark.something(...) call decorators like @pytest.mark.parametrize(...)
            ast::Expr::Call(ast::ExprCall { func, .. }) => {
                self.get_decorator_name(func)
            }
            // Handle pytest.mark.something decorators
            ast::Expr::Attribute(ExprAttribute { attr, value, .. }) => {
                let attr_str = attr.to_string();
                if let Some(inner) = self.get_decorator_name(value) {
                    // Build qualified name: inner.attr (e.g., "pytest.mark.slow")
                    Some(format!("{}.{}", inner, attr_str))
                } else {
                    // Just the attribute part (e.g., "mark.slow")
                    Some(attr_str)
                }
            }
            ast::Expr::Name(ExprName { id, .. }) => Some(id.to_string()),
            _ => None,
        }
    }

    /// Recursively extract tests from statements.
    fn extract_stmt_items(
        &self,
        stmts: &[ast::Stmt],
        class_name: &str,
        file_path: &str,
        conftest_fixtures: &Vec<String>,
        tests: &mut Vec<NativeTestNode>,
    ) {
        for stmt in stmts {
            match stmt {
                ast::Stmt::FunctionDef(func) => {
                    let fn_name = &func.name;

                    // Check if it's a test function (starts with test_)
                    if fn_name.starts_with("test_") || fn_name.starts_with("Test") {
                        // Line number extraction from AST requires source location mapping
                        let line_number = 0;
                        let node_id = format!("{}::{}", file_path, fn_name);

                        let markers = self.extract_pytest_markers(&func.decorator_list);

                        // Check if test uses external fixtures
                        let uses_external_fixtures =
                            self.check_pytest_fixture_usage(func, conftest_fixtures);

                        tests.push(NativeTestNode {
                            node_id,
                            file_path: file_path.to_string(),
                            name: fn_name.to_string(),
                            class_name: if class_name.is_empty() {
                                None
                            } else {
                                Some(class_name.to_string())
                            },
                            line_number,
                            markers,
                            is_simple: !uses_external_fixtures,
                            parameters: Vec::new(),
                        });
                    }
                }
                ast::Stmt::ClassDef(class_def) => {
                    let class_name_str = &class_def.name;

                    // Check if it's a test class
                    if class_name_str.starts_with("Test") {
                        // Extract tests from class methods
                        self.extract_stmt_items(
                            &class_def.body,
                            class_name_str,
                            file_path,
                            conftest_fixtures,
                            tests,
                        );
                    }
                }
                ast::Stmt::For(_) => {
                    // Handle for loops that might contain tests
                }
                ast::Stmt::If(_) => {
                    // Handle if statements that might contain tests
                }
                _ => {}
            }
        }
    }

    /// Extract pytest markers from decorator list.
    fn extract_pytest_markers(&self, decorators: &[ast::Expr]) -> Vec<String> {
        let mut markers = Vec::new();

        for decorator in decorators {
            if let Some(name) = self.get_decorator_name(decorator) {
                // Skip builtin fixtures
                if self.builtin_fixtures.contains(&name.as_str()) {
                    continue;
                }

                // Check for pytest.mark decorators
                if name.starts_with("pytest.mark.") {
                    let marker = name.replace("pytest.mark.", "");
                    markers.push(marker);
                } else if name == "pytest.mark.parametrize" {
                    // Handle parametrize specially
                    markers.push("parametrize".to_string());
                }
                // Also add non-pytest markers (like custom markers)
                else if !name.starts_with("pytest.") {
                    markers.push(name);
                }
            }
        }

        markers
    }

    /// Check if a function uses external fixtures.
    fn check_pytest_fixture_usage(
        &self,
        func: &ast::StmtFunctionDef,
        conftest_fixtures: &Vec<String>,
    ) -> bool {
        // Check function parameters for fixture names
        for arg in &func.args.args {
            let arg_name = &arg.def.arg.to_string();
            if conftest_fixtures.contains(arg_name) {
                return true;
            }
        }
        false
    }
}

/// Check if a file is a Python test file.
pub fn is_test_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        if ext != "py" {
            return false;
        }
    } else {
        return false;
    }

    if let Some(file_name) = path.file_name() {
        let name = file_name.to_string_lossy();
        name.starts_with("test_") || name.ends_with("_test.py")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_collect_simple() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_example.py");
        fs::write(
            &test_file,
            r#"
def test_simple():
    assert True

def test_another():
    assert 1 + 1 == 2

class TestClass:
    def test_method(self):
        assert True
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 3 tests using rustpython-parser
        assert_eq!(tests.len(), 3);

        // Check test names
        let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
        assert!(test_names.contains(&"test_simple"));
        assert!(test_names.contains(&"test_another"));
        assert!(test_names.contains(&"test_method"));
    }

    #[test]
    fn test_ignore_non_test_files() {
        let dir = TempDir::new().unwrap();

        // Create a non-test file
        let regular_file = dir.path().join("regular.py");
        fs::write(&regular_file, "def regular_func():\n    pass").unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should not find any tests (no test_ prefix)
        assert!(tests.is_empty());
    }

    #[test]
    fn test_collect_with_markers() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_marked.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.slow
def test_expensive():
    pass

@pytest.mark.parametrize("x", [1, 2, 3])
def test_param(x):
    pass
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 2 tests
        assert_eq!(tests.len(), 2);

        // Check markers
        let slow_test = tests.iter().find(|t| t.name == "test_expensive").unwrap();
        assert!(slow_test.markers.contains(&"slow".to_string()));

        let param_test = tests.iter().find(|t| t.name == "test_param").unwrap();
        assert!(param_test.markers.contains(&"parametrize".to_string()));
    }
}
