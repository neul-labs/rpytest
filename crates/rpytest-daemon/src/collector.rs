//! Native AST-based test collection using RustPython parser.
//!
//! This collector uses pure Rust AST parsing to find tests without importing pytest.
//! It's faster than pytest collection but may not detect all tests that pytest does.

use crate::error::Result;
use crate::models::{NativeTestNode, ParameterizedTestNode};
use rayon::prelude::*;
use rustpython_ast::{self as ast, ExprAttribute, ExprName};
use rustpython_parser::Parse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, warn};

// Type aliases to reduce complexity of deeply nested types
/// Information about a parameter value: (values, custom_id, marks)
type ParamValueInfo = (Vec<String>, Option<String>, Vec<String>);
/// A single parameter combination: (param_names, param_values, custom_id, marks)
type ParamCombination = (Vec<String>, Vec<String>, Option<String>, Vec<String>);

/// Information extracted from a single parameter value (for pytest.param)
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ParamValue {
    values: Vec<String>, // Parameter values
    id: Option<String>,  // Custom ID from id= keyword
    marks: Vec<String>,  // Marks from marks= keyword
}

/// Information about skip/xfail decoration
#[derive(Debug, Default, Clone)]
struct SkipXfailInfo {
    skip: bool,
    skip_reason: Option<String>,
    skipif: bool,
    skipif_condition: Option<String>,
    xfail: bool,
    xfail_reason: Option<String>,
    xfail_condition: Option<String>,
    xfail_strict: bool,
    xfail_raises: Option<String>,
}

/// Context for processing test statements within a class or file
struct TestProcessingContext<'a> {
    file_path: &'a str,
    class_name: &'a str,
    class_markers: &'a [String],
    source: &'a str,
}

/// Context for creating test nodes
struct TestCreationContext<'a> {
    fn_name: &'a str,
    line_number: u32,
    markers: &'a [String],
    uses_external_fixtures: bool,
    skip_xfail: SkipXfailInfo,
}

/// Decorators that indicate a method is not a test function
const SKIP_DECORATORS: &[&str] = &["staticmethod", "classmethod", "property", "abstractmethod"];

/// Category of a parameter value for ID generation
#[derive(Debug, Clone, PartialEq)]
enum ValueCategory {
    Dict,
    List,
    Simple,
}

/// Native test collector using RustPython AST parsing.
#[derive(Debug, Clone)]
pub struct NativeCollector {
    /// Root path of the repository
    repo_path: PathBuf,
    /// Built-in pytest fixtures that require pytest
    builtin_fixtures: Vec<&'static str>,
    /// Collected tests (shared for thread safety)
    tests: Arc<Mutex<Vec<NativeTestNode>>>,
    /// Parametrized fixtures: fixture_name -> list of param values
    fixture_params: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl Default for NativeCollector {
    fn default() -> Self {
        NativeCollector {
            repo_path: PathBuf::from("."),
            builtin_fixtures: vec![
                "capsys",
                "capfd",
                "capsysbinary",
                "capfdbinary",
                "caplog",
                "tmp_path",
                "tmp_path_factory",
                "tmpdir",
                "tmpdir_factory",
                "request",
                "pytestconfig",
                "cache",
                "recwarn",
                "monkeypatch",
                "doctest_namespace",
            ],
            tests: Arc::new(Mutex::new(Vec::new())),
            fixture_params: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl NativeCollector {
    /// Create a new collector for the given repository.
    pub fn new(repo_path: &Path) -> Self {
        NativeCollector {
            repo_path: repo_path.to_path_buf(),
            builtin_fixtures: vec![
                "capsys",
                "capfd",
                "capsysbinary",
                "capfdbinary",
                "caplog",
                "tmp_path",
                "tmp_path_factory",
                "tmpdir",
                "tmpdir_factory",
                "request",
                "pytestconfig",
                "cache",
                "recwarn",
                "monkeypatch",
                "doctest_namespace",
            ],
            tests: Arc::new(Mutex::new(Vec::new())),
            fixture_params: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Collect all tests using AST parsing.
    pub fn collect(&self) -> Result<Vec<NativeTestNode>> {
        self.fixture_params.lock().unwrap().clear();

        // Find all test files
        let test_files = self.find_test_files()?;

        // Parse files in parallel and collect results without mutex contention
        // Each thread returns its own Vec, then we flatten at the end
        let all_tests: Vec<NativeTestNode> = test_files
            .par_iter()
            .filter_map(|file| self.parse_test_file(file).ok())
            .flatten()
            .collect();

        // Store the collected tests for fixture expansion
        {
            let mut tests = self.tests.lock().unwrap();
            tests.clear();
            tests.extend(all_tests);
        }

        // Post-process: expand tests based on fixture parametrization
        let expanded_tests = self.expand_tests_by_fixture_params();

        debug!("Collected {} native tests", expanded_tests.len());
        Ok(expanded_tests)
    }

    /// Expand tests based on fixture parametrization.
    /// If a test uses a parametrized fixture, create one variant per fixture param.
    fn expand_tests_by_fixture_params(&self) -> Vec<NativeTestNode> {
        let tests = self.tests.lock().unwrap().clone();
        let fixture_params = self.fixture_params.lock().unwrap().clone();

        if fixture_params.is_empty() {
            return tests;
        }

        let mut expanded = Vec::new();
        for test in tests {
            // Check if this test uses any parametrized fixtures
            let fixture_params_for_test = self.get_fixture_params_for_test(&test, &fixture_params);

            if fixture_params_for_test.is_empty() {
                // No parametrized fixtures used, keep the test as-is
                expanded.push(test);
            } else if fixture_params_for_test.len() == 1 {
                // Single parametrized fixture - expand into multiple variants
                let (fixture_name, param_values) = &fixture_params_for_test[0];
                for param_value in param_values.iter() {
                    let mut variant = test.clone();
                    // Use the actual param value as ID (like pytest does)
                    variant.node_id = format!("{}[{}]", test.node_id, param_value);
                    // Add marker indicating fixture parametrization
                    variant
                        .markers
                        .push(format!("fixture_param:{}={}", fixture_name, param_value));
                    expanded.push(variant);
                }
            } else {
                // Multiple parametrized fixtures - create cartesian product
                let combinations =
                    self.cartesian_product_of_fixture_params(&fixture_params_for_test);
                for combo in combinations {
                    let mut variant = test.clone();
                    // Create ID from all param values
                    let id_parts: Vec<String> = combo
                        .iter()
                        .enumerate()
                        .map(|(idx, (_, param_value))| format!("{}={}", idx + 1, param_value))
                        .collect();
                    variant.node_id = format!("{}[{}]", test.node_id, id_parts.join("-"));
                    expanded.push(variant);
                }
            }
        }

        expanded
    }

    /// Get parametrized fixtures used by a test.
    fn get_fixture_params_for_test(
        &self,
        test: &NativeTestNode,
        fixture_params: &HashMap<String, Vec<String>>,
    ) -> Vec<(String, Vec<String>)> {
        let mut result = Vec::new();

        // Check each fixture param to see if the test uses it
        for (fixture_name, param_values) in fixture_params {
            // The test uses a fixture if the fixture name appears in its markers
            // or if we can infer it from the test structure
            // For now, check if any marker indicates fixture usage
            let expected_marker = format!("uses_fixture:{}", fixture_name);
            for marker in &test.markers {
                if marker == &expected_marker {
                    result.push((fixture_name.clone(), param_values.clone()));
                    break;
                }
            }
        }

        result
    }

    /// Compute cartesian product of fixture param values.
    fn cartesian_product_of_fixture_params(
        &self,
        fixture_params: &[(String, Vec<String>)],
    ) -> Vec<Vec<(String, String)>> {
        if fixture_params.is_empty() {
            return Vec::new();
        }

        if fixture_params.len() == 1 {
            return fixture_params[0]
                .1
                .iter()
                .map(|p| vec![(fixture_params[0].0.clone(), p.clone())])
                .collect();
        }

        // Recursive cartesian product
        fn compute_product(
            params: &[(String, Vec<String>)],
            index: usize,
        ) -> Vec<Vec<(String, String)>> {
            if index >= params.len() {
                return vec![Vec::new()];
            }

            let (name, values) = &params[index];
            let rest = compute_product(params, index + 1);

            let mut result = Vec::new();
            for value in values {
                for mut combo in rest.clone() {
                    combo.insert(0, (name.clone(), value.clone()));
                    result.push(combo);
                }
            }

            result
        }

        compute_product(fixture_params, 0)
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
            if path_str.contains("/.venv/")
                || path_str.contains("/venv/")
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
                    if file_name.starts_with("test_") || file_name.ends_with("_test.py") {
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
        let syntax = ast::Suite::parse(&content, "<test>");

        match syntax {
            Ok(stmts) => self.extract_tests_from_ast(&stmts, path, &content),
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
                Ok(Vec::new())
            }
        }
    }

    /// Convert a byte offset to a 1-based line number.
    fn byte_offset_to_line(&self, source: &str, offset: usize) -> u32 {
        let offset = offset.min(source.len());
        let line = source[..offset].chars().filter(|&c| c == '\n').count() + 1;
        line as u32
    }

    /// Extract test nodes from Python AST statements.
    fn extract_tests_from_ast(
        &self,
        stmts: &[ast::Stmt],
        path: &Path,
        source: &str,
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

        // Also collect fixtures from this file (for fixture parametrization detection)
        let file_fixtures = self.extract_conftest_fixtures(stmts);

        // Merge conftest and file-local fixtures
        let mut all_fixtures = conftest_fixtures;
        for fixture in &file_fixtures {
            if !all_fixtures.contains(fixture) {
                all_fixtures.push(fixture.clone());
            }
        }

        // Extract tests from statements
        self.extract_stmt_items(stmts, "", &relative_path, &all_fixtures, source, &mut tests);

        Ok(tests)
    }

    /// Extract fixtures from conftest.py (handles both sync and async)
    fn extract_conftest_fixtures(&self, stmts: &[ast::Stmt]) -> Vec<String> {
        let mut fixtures = Vec::new();

        for stmt in stmts {
            match stmt {
                ast::Stmt::FunctionDef(func) => {
                    self.check_for_fixture(func, &mut fixtures);
                }
                ast::Stmt::AsyncFunctionDef(func) => {
                    self.check_for_async_fixture(func, &mut fixtures);
                }
                _ => {}
            }
        }

        fixtures
    }

    /// Check a function def for @pytest.fixture decorator
    fn check_for_fixture(&self, func: &ast::StmtFunctionDef, fixtures: &mut Vec<String>) {
        for decorator in &func.decorator_list {
            if let Some(name) = self.get_decorator_name(decorator) {
                // Handle both @fixture and @pytest.fixture
                if name == "fixture" || name == "pytest.fixture" {
                    fixtures.push(func.name.to_string());
                    // Also check for params
                    self.extract_fixture_params(decorator, &func.name);
                }
            }
        }
    }

    /// Extract parameters from @pytest.fixture(params=[...]) decorator
    fn extract_fixture_params(&self, decorator: &ast::Expr, fixture_name: &str) {
        // Handle both @fixture and @fixture(params=[...])
        let keywords = match decorator {
            ast::Expr::Call(ast::ExprCall { keywords, .. }) => keywords,
            _ => return,
        };

        // Look for params keyword argument
        for keyword in keywords {
            if keyword.arg.as_deref() == Some("params") {
                // Extract param values
                if let Some(param_values) = self.extract_list_param_values(&keyword.value) {
                    let mut fixture_params = self.fixture_params.lock().unwrap();
                    fixture_params.insert(fixture_name.to_string(), param_values);
                }
            }
        }
    }

    /// Extract parameter values from a list expression (for fixture params)
    fn extract_list_param_values(&self, expr: &ast::Expr) -> Option<Vec<String>> {
        match expr {
            ast::Expr::List(ast::ExprList { elts, .. }) => {
                let mut values = Vec::new();
                for elt in elts {
                    if let ast::Expr::Constant(ast::ExprConstant { value, .. }) = elt {
                        values.push(self.format_constant(value));
                    } else {
                        // Non-constant element - use repr
                        values.push(format!("{:?}", elt));
                    }
                }
                Some(values)
            }
            _ => None,
        }
    }

    /// Check an async function def for @pytest.fixture decorator
    fn check_for_async_fixture(
        &self,
        func: &ast::StmtAsyncFunctionDef,
        fixtures: &mut Vec<String>,
    ) {
        for decorator in &func.decorator_list {
            if let Some(name) = self.get_decorator_name(decorator) {
                // Handle both @fixture and @pytest.fixture
                if name == "fixture" || name == "pytest.fixture" {
                    fixtures.push(func.name.to_string());
                    // Also check for params
                    self.extract_fixture_params(decorator, &func.name);
                }
            }
        }
    }

    /// Get decorator name from a decorator expression.
    fn get_decorator_name(&self, decorator: &ast::Expr) -> Option<String> {
        match decorator {
            // Handle pytest.mark.something(...) call decorators like @pytest.mark.parametrize(...)
            ast::Expr::Call(ast::ExprCall { func, .. }) => self.get_decorator_name(func),
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

    /// Extract parametrize info from decorator arguments.
    fn extract_parametrize_info(&self, decorator: &ast::Expr) -> Option<ParameterizedTestNode> {
        // Look for @pytest.mark.parametrize(args)
        if let ast::Expr::Call(ast::ExprCall { func, args, .. }) = decorator {
            let func_name = self.get_decorator_name(func);
            if func_name.as_deref() != Some("pytest.mark.parametrize") {
                return None;
            }

            // args[0] should be the parameter names string (e.g., "x" or "x,y")
            // args[1] should be the list of values
            if args.len() < 2 {
                return None;
            }

            // Extract parameter names from first argument
            let param_names: Vec<String> = match &args[0] {
                ast::Expr::Constant(ast::ExprConstant {
                    value: ast::Constant::Str(s),
                    ..
                }) => s.split(',').map(|s| s.trim().to_string()).collect(),
                _ => return None,
            };

            // Extract parameter values from second argument (with custom IDs and marks)
            let param_values_with_info = self.extract_param_values_with_info(&args[1])?;

            // Convert to the format expected by ParameterizedTestNode
            // Now stores (values, custom_id, marks) tuples
            let param_values: Vec<(Vec<String>, Option<String>, Vec<String>)> =
                param_values_with_info
                    .iter()
                    .map(|(values, custom_id, marks)| {
                        (values.clone(), custom_id.clone(), marks.clone())
                    })
                    .collect();

            Some(ParameterizedTestNode {
                param_names,
                param_values,
                test_id: String::new(),
            })
        } else {
            None
        }
    }

    /// Extract parameter values from a parametrize argument list.
    #[allow(dead_code)]
    fn extract_param_values(&self, expr: &ast::Expr) -> Option<Vec<Vec<String>>> {
        match expr {
            // List of values: [1, 2, 3] or [(1, 2), (3, 4)]
            ast::Expr::List(ast::ExprList { elts, .. }) => {
                let mut all_values = Vec::new();
                for elt in elts {
                    if let Some((values, _, _)) = self.extract_single_param_value_full(elt) {
                        all_values.push(values);
                    } else {
                        return None;
                    }
                }
                Some(all_values)
            }
            _ => None,
        }
    }

    /// Extract a single parameter value (can be a scalar or tuple).
    /// Returns the values, custom ID, and marks from pytest.param.
    fn extract_single_param_value_full(
        &self,
        expr: &ast::Expr,
    ) -> Option<(Vec<String>, Option<String>, Vec<String>)> {
        match expr {
            // Simple value: 1, "string", True, None
            ast::Expr::Constant(ast::ExprConstant { value, .. }) => {
                Some((vec![self.format_constant(value)], None, Vec::new()))
            }
            // Tuple: (1, 2, 3)
            ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                let mut values = Vec::new();
                for elt in elts {
                    if let ast::Expr::Constant(ast::ExprConstant { value, .. }) = elt {
                        values.push(self.format_constant(value));
                    } else {
                        // Non-constant tuple element - represent as repr
                        values.push(format!("{:?}", elt));
                    }
                }
                Some((values, None, Vec::new()))
            }
            // Dict: {"a": 1} - use compact representation
            ast::Expr::Dict(ast::ExprDict { keys, values, .. }) => {
                // Generate a compact representation like pytest does
                let parts: Vec<String> = keys
                    .iter()
                    .zip(values.iter())
                    .filter_map(|(k, v)| {
                        // Collapse nested if-let into single match
                        if let (
                            Some(k_const),
                            ast::Expr::Constant(ast::ExprConstant { value, .. }),
                        ) = (
                            k.as_ref().and_then(|k| {
                                if let ast::Expr::Constant(ast::ExprConstant {
                                    value: ast::Constant::Str(s),
                                    ..
                                }) = k
                                {
                                    Some(s)
                                } else {
                                    None
                                }
                            }),
                            v,
                        ) {
                            Some(format!("{}={}", k_const, self.format_constant(value)))
                        } else {
                            None
                        }
                    })
                    .collect();
                let repr = if parts.is_empty() {
                    "{}".to_string()
                } else {
                    format!("{{{}}}", parts.join(","))
                };
                Some((vec![repr], None, Vec::new()))
            }
            // List: [1, 2, 3] - use compact representation
            ast::Expr::List(ast::ExprList { elts, .. }) => {
                let parts: Vec<String> = elts
                    .iter()
                    .filter_map(|elt| {
                        if let ast::Expr::Constant(ast::ExprConstant { value, .. }) = elt {
                            Some(self.format_constant(value))
                        } else {
                            None
                        }
                    })
                    .collect();
                let repr = if parts.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", parts.join(","))
                };
                Some((vec![repr], None, Vec::new()))
            }
            // pytest.param: pytest.param(1, id="...", marks=pytest.mark.skip(...))
            ast::Expr::Call(ast::ExprCall {
                func,
                args,
                keywords,
                ..
            }) => {
                // Check if this is a call to pytest.param
                // The func could be:
                // - Name("param") - direct param() call
                // - Attribute(attr="param", value=Name("pytest")) - pytest.param()
                let is_param = match func.as_ref() {
                    ast::Expr::Name(name_expr) => name_expr.id.as_str() == "param",
                    ast::Expr::Attribute(attr) => {
                        if attr.attr.as_str() == "param" {
                            if let ast::Expr::Name(name_expr) = &*attr.value {
                                name_expr.id.as_str() == "param"
                                    || name_expr.id.as_str() == "pytest"
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if is_param {
                    // Extract custom ID from keywords
                    let custom_id = keywords
                        .iter()
                        .find(|kw| kw.arg.as_deref() == Some("id"))
                        .and_then(|kw| {
                            if let ast::Expr::Constant(ast::ExprConstant {
                                value: ast::Constant::Str(s),
                                ..
                            }) = &kw.value
                            {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });

                    // Extract marks from keywords
                    let marks = self.parse_marks_from_keywords(keywords);

                    // Extract value(s) from args
                    if args.is_empty() {
                        return Some((vec!["param".to_string()], custom_id, marks));
                    }
                    // First arg is the value
                    if let Some((values, _, _)) = self.extract_single_param_value_full(&args[0]) {
                        return Some((values, custom_id, marks));
                    }
                }
                None
            }
            // UnaryOp: -0, -0.0, -1, etc.
            ast::Expr::UnaryOp(ast::ExprUnaryOp { op, operand, .. }) => {
                match (op, operand.as_ref()) {
                    (ast::UnaryOp::USub, ast::Expr::Constant(ast::ExprConstant { value, .. })) => {
                        // Negative constant: -0, -0.0, -1, etc.
                        // Special case: -0 (integer) evaluates to 0 in Python
                        if let ast::Constant::Int(n) = value {
                            if n.to_string() == "0" {
                                // -0 == 0 in Python, so just return 0
                                return Some((vec!["0".to_string()], None, Vec::new()));
                            }
                        }
                        let formatted = self.format_constant(value);
                        Some((vec![format!("-{}", formatted)], None, Vec::new()))
                    }
                    (ast::UnaryOp::UAdd, ast::Expr::Constant(ast::ExprConstant { value, .. })) => {
                        // Positive constant: +0, +1, etc. (rare but possible)
                        Some((vec![self.format_constant(value)], None, Vec::new()))
                    }
                    (ast::UnaryOp::Not, _) => {
                        // not expression - use repr
                        Some((vec![format!("{:?}", expr)], None, Vec::new()))
                    }
                    _ => Some((vec![format!("{:?}", expr)], None, Vec::new())),
                }
            }
            // For other expressions, use debug representation
            _ => Some((vec![format!("{:?}", expr)], None, Vec::new())),
        }
    }

    /// Parse marks from pytest.param keywords.
    fn parse_marks_from_keywords(&self, keywords: &[ast::Keyword]) -> Vec<String> {
        let mut marks = Vec::new();

        for keyword in keywords {
            if keyword.arg.as_deref() == Some("marks") {
                // Parse the marks expression
                self.extract_marks_from_expr(&keyword.value, &mut marks);
            }
        }

        marks
    }

    /// Recursively extract marks from an expression.
    fn extract_marks_from_expr(&self, expr: &ast::Expr, marks: &mut Vec<String>) {
        match expr {
            // Single mark: pytest.mark.skip
            ast::Expr::Call(call) => {
                if let Some(name) = self.get_decorator_name(&call.func) {
                    let marker = name.replace("pytest.mark.", "");
                    marks.push(marker);
                }
            }
            // List of marks: [pytest.mark.skip, pytest.mark.slow]
            ast::Expr::List(list) => {
                for elt in &list.elts {
                    self.extract_marks_from_expr(elt, marks);
                }
            }
            // Attribute access: pytest.mark.skip
            ast::Expr::Attribute(_attr) => {
                if let Some(name) = self.get_decorator_name(expr) {
                    let marker = name.replace("pytest.mark.", "");
                    marks.push(marker);
                }
            }
            _ => {}
        }
    }

    /// Extract parameter values from a parametrize argument list, including custom IDs and marks.
    /// Returns Vec<ParamValueInfo> where each element is (values, custom_id, marks)
    fn extract_param_values_with_info(&self, expr: &ast::Expr) -> Option<Vec<ParamValueInfo>> {
        match expr {
            // List of values: [1, 2, 3] or [(1, 2), (3, 4)]
            ast::Expr::List(ast::ExprList { elts, .. }) => {
                let mut all_values = Vec::new();
                for elt in elts {
                    if let Some((values, custom_id, marks)) =
                        self.extract_single_param_value_full(elt)
                    {
                        all_values.push((values, custom_id, marks));
                    } else {
                        return None;
                    }
                }
                Some(all_values)
            }
            _ => None,
        }
    }

    /// Format a Python constant for display in test ID.
    fn format_constant(&self, value: &ast::Constant) -> String {
        match value {
            ast::Constant::Str(s) => {
                // For test IDs, just return the string as-is (pytest does the same)
                // Special characters will be handled by format_param_value if needed
                s.clone()
            }
            ast::Constant::Int(n) => n.to_string(),
            ast::Constant::Float(n) => {
                let s = n.to_string();
                // Clean up float representation
                if s.contains('e') || s.contains('E') {
                    format!("{:?}", n)
                } else if !s.contains('.') {
                    // Ensure floats have decimal point (e.g., 0.0 not 0)
                    format!("{}.0", s)
                } else {
                    s
                }
            }
            ast::Constant::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            ast::Constant::None => "None".to_string(),
            _ => format!("{:?}", value),
        }
    }

    /// Format a parameter value for test ID.
    /// Match pytest's ID format: type prefix + index for containers, value for simple types.
    fn format_param_value(&self, value: &str) -> String {
        // Detect type and format accordingly
        if value == "{}" {
            // Empty dict - pytest uses dctN prefix
            "dct".to_string()
        } else if value.starts_with('{') {
            // Dict with content - pytest uses dctN prefix
            "dct".to_string()
        } else if value == "[]" {
            // Empty list - pytest uses lstN prefix
            "lst".to_string()
        } else if value.starts_with('[') {
            // List with content - pytest uses lstN prefix
            "lst".to_string()
        } else if value == "None" {
            // Python None - use capital N like pytest
            "None".to_string()
        } else if value == "True" || value == "False" {
            // Booleans - pytest uses capitalized True/False
            value.to_string()
        } else {
            // Match pytest behavior: preserve most characters, only escape those that break node_id parsing
            // pytest only needs to escape: [ ] / :: (node_id separators)
            // Convert escape sequences to literal backslash representation (what pytest shows)
            let sanitized = value
                .replace('[', "_")
                .replace(']', "_")
                .replace("::", "_")
                .replace('/', "_")
                // Convert actual escape sequences to their escaped representation
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t")
                // Backslash itself should be escaped
                .replace('\\', "\\");
            // Limit length
            if sanitized.len() > 30 {
                format!("{}...", &sanitized[..27])
            } else {
                sanitized
            }
        }
    }

    /// Categorize a value for ID generation purposes.
    fn categorize_value(&self, value: &str) -> ValueCategory {
        if value == "{}" || value.starts_with('{') {
            ValueCategory::Dict
        } else if value == "[]" || value.starts_with('[') {
            ValueCategory::List
        } else {
            ValueCategory::Simple
        }
    }

    /// Check if parameter values have mixed types.
    /// Returns true if there's a mix of container types (list/dict) with simple types.
    fn has_mixed_param_types(
        &self,
        param_values: &[(Vec<String>, Option<String>, Vec<String>)],
    ) -> bool {
        let mut has_dict = false;
        let mut has_list = false;
        let mut has_simple = false;

        for (values, _, _) in param_values {
            for value in values {
                match self.categorize_value(value) {
                    ValueCategory::Dict => has_dict = true,
                    ValueCategory::List => has_list = true,
                    ValueCategory::Simple => has_simple = true,
                }
            }
        }

        // Mixed if we have containers (dict or list) AND simple types
        // OR if we have both dicts AND lists
        (has_dict || has_list) && has_simple
    }

    /// Deduplicate IDs by adding _0, _1, etc. suffixes for duplicates.
    /// This matches pytest's behavior when parameter values evaluate to the same thing.
    fn deduplicate_ids(&self, ids: &[String]) -> Vec<String> {
        let mut result = Vec::with_capacity(ids.len());
        let mut seen: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();

        // First pass: group indices by ID
        for (idx, id) in ids.iter().enumerate() {
            seen.entry(id.as_str()).or_default().push(idx);
        }

        // Second pass: generate final IDs
        let mut counters: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for id in ids {
            let indices = &seen[id.as_str()];
            if indices.len() > 1 {
                // This ID appears multiple times, add suffix
                let counter = counters.entry(id.as_str()).or_insert(0);
                result.push(format!("{}_{}", id, counter));
                *counter += 1;
            } else {
                // Unique ID, use as-is
                result.push(id.clone());
            }
        }

        result
    }

    /// Generate pytest-style test ID from parameter values and index.
    /// Pytest uses: type_prefix + index for containers, or just value for simple types.
    fn generate_pytest_id(&self, values: &[String], idx: usize) -> String {
        self.generate_pytest_id_with_context(values, idx, false)
    }

    /// Generate pytest-style test ID with context about mixed types.
    /// When use_value_for_containers is true, uses 'value' prefix for lists/dicts (for mixed-type params).
    fn generate_pytest_id_with_context(
        &self,
        values: &[String],
        idx: usize,
        use_value_for_containers: bool,
    ) -> String {
        if values.len() == 1 {
            // Single parameter - use format_param_value for the value
            let formatted = if use_value_for_containers {
                self.format_param_value_with_context(&values[0], true)
            } else {
                self.format_param_value(&values[0])
            };
            // For containers (dict, list, value), use prefix + index
            // For simple types (int, str, None), use value directly
            if formatted == "dct" || formatted == "lst" || formatted == "value" {
                format!("{}{}", formatted, idx)
            } else {
                formatted
            }
        } else {
            // Multiple parameters - format each and join with hyphens
            let formatted: Vec<String> = values
                .iter()
                .map(|v| {
                    if use_value_for_containers {
                        self.format_param_value_with_context(v, true)
                    } else {
                        self.format_param_value(v)
                    }
                })
                .collect();
            formatted.join("-")
        }
    }

    /// Format a parameter value with context about whether to use 'value' prefix for containers.
    fn format_param_value_with_context(
        &self,
        value: &str,
        use_value_for_containers: bool,
    ) -> String {
        if use_value_for_containers {
            // In mixed-type context, use 'value' prefix for containers
            if value == "{}" || value.starts_with('{') || value == "[]" || value.starts_with('[') {
                return "value".to_string();
            }
        }
        self.format_param_value(value)
    }

    /// Extract pytest markers from a decorator list (for class-level markers).
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
                }
            }
        }

        markers
    }

    /// Recursively extract tests from statements with class marker inheritance.
    fn extract_stmt_items(
        &self,
        stmts: &[ast::Stmt],
        class_name: &str,
        file_path: &str,
        conftest_fixtures: &[String],
        source: &str,
        tests: &mut Vec<NativeTestNode>,
    ) {
        for stmt in stmts {
            match stmt {
                // Handle regular function definitions
                ast::Stmt::FunctionDef(func) => {
                    self.process_function_def(
                        func,
                        class_name,
                        file_path,
                        conftest_fixtures,
                        source,
                        tests,
                    );
                }
                // Handle async function definitions
                ast::Stmt::AsyncFunctionDef(func) => {
                    self.process_async_function_def(
                        func,
                        class_name,
                        file_path,
                        conftest_fixtures,
                        source,
                        tests,
                    );
                }
                ast::Stmt::ClassDef(class_def) => {
                    let class_name_str = &class_def.name;

                    // Pytest only collects test methods from classes starting with "Test"
                    // (not Base*, Mixin*, Abstract*, etc.)
                    if class_name_str.starts_with("Test") {
                        // Extract class-level markers
                        let class_markers = self.extract_pytest_markers(&class_def.decorator_list);

                        // Extract tests from class methods with inherited markers
                        let ctx = TestProcessingContext {
                            file_path,
                            class_name: class_name_str,
                            class_markers: &class_markers,
                            source,
                        };
                        self.extract_stmt_items_with_class_markers(
                            &class_def.body,
                            &ctx,
                            conftest_fixtures,
                            tests,
                        );

                        // Also collect inherited test methods from base classes
                        // Extract base class names and look for them in the same file
                        let base_class_names = self.extract_base_class_names(class_def);
                        for base_name in base_class_names {
                            // Find the base class definition in the same file
                            if let Some(base_class) = self.find_class_in_stmts(stmts, &base_name) {
                                // Collect test methods from base class (recursively handles inheritance)
                                self.collect_inherited_tests(
                                    base_class,
                                    class_name_str, // Use the derived class name
                                    &class_markers,
                                    file_path,
                                    conftest_fixtures,
                                    source,
                                    tests,
                                    stmts, // Pass stmts for recursive base class lookup
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract tests from class body with inherited class markers.
    fn extract_stmt_items_with_class_markers(
        &self,
        stmts: &[ast::Stmt],
        ctx: &TestProcessingContext,
        conftest_fixtures: &[String],
        tests: &mut Vec<NativeTestNode>,
    ) {
        for stmt in stmts {
            match stmt {
                ast::Stmt::FunctionDef(func) => {
                    self.process_function_def_with_class_markers(
                        func,
                        ctx,
                        conftest_fixtures,
                        tests,
                    );
                }
                ast::Stmt::AsyncFunctionDef(func) => {
                    self.process_async_function_def_with_class_markers(
                        func,
                        ctx,
                        conftest_fixtures,
                        tests,
                    );
                }
                ast::Stmt::ClassDef(nested_class) => {
                    // Handle nested test classes
                    // Pytest only collects test methods from classes starting with "Test"
                    if nested_class.name.starts_with("Test") {
                        let nested_class_markers =
                            self.extract_pytest_markers(&nested_class.decorator_list);
                        // Combine outer and inner class markers
                        let mut combined_markers = ctx.class_markers.to_vec();
                        combined_markers.extend(nested_class_markers);

                        let nested_ctx = TestProcessingContext {
                            file_path: ctx.file_path,
                            class_name: &nested_class.name,
                            class_markers: &combined_markers,
                            source: ctx.source,
                        };
                        self.extract_stmt_items_with_class_markers(
                            &nested_class.body,
                            &nested_ctx,
                            conftest_fixtures,
                            tests,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// Process a regular function definition and extract tests.
    fn process_function_def(
        &self,
        func: &ast::StmtFunctionDef,
        class_name: &str,
        file_path: &str,
        conftest_fixtures: &[String],
        source: &str,
        tests: &mut Vec<NativeTestNode>,
    ) {
        let ctx = TestProcessingContext {
            file_path,
            class_name,
            class_markers: &[],
            source,
        };
        self.process_function_def_with_class_markers(func, &ctx, conftest_fixtures, tests);
    }

    /// Process a function definition with inherited class markers.
    fn process_function_def_with_class_markers(
        &self,
        func: &ast::StmtFunctionDef,
        ctx: &TestProcessingContext,
        conftest_fixtures: &[String],
        tests: &mut Vec<NativeTestNode>,
    ) {
        let fn_name = &func.name;

        // Check if it's a test function (starts with test_)
        if fn_name.starts_with("test_") || fn_name.starts_with("Test") {
            let decorators = &func.decorator_list;

            // Skip methods decorated with @staticmethod, @classmethod, @property, @abstractmethod
            // These are not valid test functions even if they start with test_
            let has_skip_decorator = decorators.iter().any(|d| {
                if let Some(name) = self.get_decorator_name(d) {
                    SKIP_DECORATORS.contains(&name.as_str())
                } else {
                    false
                }
            });
            if has_skip_decorator {
                return;
            }

            let line_number = self.extract_line_number_stmt(func, ctx.source);

            // Extract markers and parametrize info
            let (method_markers, parametrize_infos) =
                self.extract_markers_and_parametrize(decorators);

            // Merge class markers with method markers
            let mut all_markers = ctx.class_markers.to_vec();
            all_markers.extend(method_markers);

            // Get fixtures used by this test and add markers for them
            let used_fixtures = self.get_pytest_fixture_usage_func(&func.args, conftest_fixtures);
            let uses_external_fixtures = !used_fixtures.is_empty();
            for fixture_name in &used_fixtures {
                all_markers.push(format!("uses_fixture:{}", fixture_name));
            }

            // Check for skip/xfail
            let skip_xfail = self.extract_skip_xfail_info(decorators);

            // Handle parametrization
            if parametrize_infos.is_empty() {
                // Single test node
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                self.create_test_node(&test_ctx, ctx.file_path, ctx.class_name, tests);
            } else if parametrize_infos.len() == 1 {
                // Single parametrize - expand as before
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                let param_info = parametrize_infos.into_iter().next().unwrap();
                self.expand_parametrized_tests(
                    &test_ctx,
                    ctx.file_path,
                    ctx.class_name,
                    param_info,
                    tests,
                );
            } else {
                // Stacked parametrize - cartesian product
                // Reverse the order since pytest applies decorators bottom-to-top
                let mut reversed_infos = parametrize_infos.clone();
                reversed_infos.reverse();
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                self.expand_stacked_parametrized_tests(
                    &test_ctx,
                    ctx.file_path,
                    ctx.class_name,
                    reversed_infos,
                    tests,
                );
            }
        }
    }

    /// Process an async function definition and extract tests.
    fn process_async_function_def(
        &self,
        func: &ast::StmtAsyncFunctionDef,
        class_name: &str,
        file_path: &str,
        conftest_fixtures: &[String],
        source: &str,
        tests: &mut Vec<NativeTestNode>,
    ) {
        let ctx = TestProcessingContext {
            file_path,
            class_name,
            class_markers: &[],
            source,
        };
        self.process_async_function_def_with_class_markers(func, &ctx, conftest_fixtures, tests);
    }

    /// Process an async function definition with inherited class markers.
    fn process_async_function_def_with_class_markers(
        &self,
        func: &ast::StmtAsyncFunctionDef,
        ctx: &TestProcessingContext,
        conftest_fixtures: &[String],
        tests: &mut Vec<NativeTestNode>,
    ) {
        let fn_name = &func.name;

        // Check if it's a test function (starts with test_)
        if fn_name.starts_with("test_") || fn_name.starts_with("Test") {
            let decorators = &func.decorator_list;

            // Skip methods decorated with @staticmethod, @classmethod, @property, @abstractmethod
            // These are not valid test functions even if they start with test_
            let has_skip_decorator = decorators.iter().any(|d| {
                if let Some(name) = self.get_decorator_name(d) {
                    SKIP_DECORATORS.contains(&name.as_str())
                } else {
                    false
                }
            });
            if has_skip_decorator {
                return;
            }

            let line_number = self.extract_line_number_async_stmt(func, ctx.source);

            // Extract markers and parametrize info
            let (method_markers, parametrize_infos) =
                self.extract_markers_and_parametrize(decorators);

            // Merge class markers with method markers
            let mut all_markers = ctx.class_markers.to_vec();
            all_markers.extend(method_markers);

            // Check if test uses external fixtures
            let uses_external_fixtures =
                self.check_pytest_fixture_usage_async(&func.args, conftest_fixtures);

            // Check for skip/xfail
            let skip_xfail = self.extract_skip_xfail_info(decorators);

            // Handle parametrization
            if parametrize_infos.is_empty() {
                // Single test node
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                self.create_test_node(&test_ctx, ctx.file_path, ctx.class_name, tests);
            } else if parametrize_infos.len() == 1 {
                // Single parametrize - expand as before
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                let param_info = parametrize_infos.into_iter().next().unwrap();
                self.expand_parametrized_tests(
                    &test_ctx,
                    ctx.file_path,
                    ctx.class_name,
                    param_info,
                    tests,
                );
            } else {
                // Stacked parametrize - cartesian product
                // Reverse the order since pytest applies decorators bottom-to-top
                let mut reversed_infos = parametrize_infos.clone();
                reversed_infos.reverse();
                let test_ctx = TestCreationContext {
                    fn_name,
                    line_number,
                    markers: &all_markers,
                    uses_external_fixtures,
                    skip_xfail,
                };
                self.expand_stacked_parametrized_tests(
                    &test_ctx,
                    ctx.file_path,
                    ctx.class_name,
                    reversed_infos,
                    tests,
                );
            }
        }
    }

    /// Create a single test node.
    fn create_test_node(
        &self,
        ctx: &TestCreationContext,
        file_path: &str,
        class_name: &str,
        tests: &mut Vec<NativeTestNode>,
    ) {
        let node_id = if class_name.is_empty() {
            format!("{}::{}", file_path, ctx.fn_name)
        } else {
            format!("{}::{}::{}", file_path, class_name, ctx.fn_name)
        };

        let mut final_markers = ctx.markers.to_vec();
        if ctx.skip_xfail.skip {
            final_markers.push("skip".to_string());
        }
        if ctx.skip_xfail.skipif {
            final_markers.push("skipif".to_string());
        }
        if ctx.skip_xfail.xfail {
            final_markers.push("xfail".to_string());
        }

        tests.push(NativeTestNode {
            node_id,
            file_path: file_path.to_string(),
            name: ctx.fn_name.to_string(),
            class_name: if class_name.is_empty() {
                None
            } else {
                Some(class_name.to_string())
            },
            line_number: ctx.line_number,
            markers: final_markers,
            is_simple: !ctx.uses_external_fixtures,
            parameters: Vec::new(),
            // Only pre-skip unconditional @pytest.mark.skip, not @pytest.mark.skipif
            // skipif conditions can only be evaluated at runtime by pytest
            skip: ctx.skip_xfail.skip,
            skip_reason: ctx
                .skip_xfail
                .skip_reason
                .clone()
                .or(ctx.skip_xfail.skipif_condition.clone()),
            xfail: ctx.skip_xfail.xfail,
            xfail_reason: ctx.skip_xfail.xfail_reason.clone(),
            xfail_strict: ctx.skip_xfail.xfail_strict,
        });
    }

    /// Extract line number from function definition.
    fn extract_line_number_stmt(&self, func: &ast::StmtFunctionDef, source: &str) -> u32 {
        let offset = func.range.start().to_usize();
        self.byte_offset_to_line(source, offset)
    }

    /// Extract line number from async function definition.
    fn extract_line_number_async_stmt(
        &self,
        func: &ast::StmtAsyncFunctionDef,
        source: &str,
    ) -> u32 {
        let offset = func.range.start().to_usize();
        self.byte_offset_to_line(source, offset)
    }

    /// Extract markers and parametrize info from decorator list.
    /// Returns all parametrize decorators (for stacked parametrize support).
    fn extract_markers_and_parametrize(
        &self,
        decorators: &[ast::Expr],
    ) -> (Vec<String>, Vec<ParameterizedTestNode>) {
        let mut markers = Vec::new();
        let mut parametrize_infos = Vec::new();

        for decorator in decorators {
            if let Some(name) = self.get_decorator_name(decorator) {
                // Skip builtin fixtures
                if self.builtin_fixtures.contains(&name.as_str()) {
                    continue;
                }

                // Check for parametrize decorator - collect ALL of them
                if name == "pytest.mark.parametrize" {
                    if let Some(info) = self.extract_parametrize_info(decorator) {
                        parametrize_infos.push(info);
                    }
                    continue;
                }

                // Check for pytest.mark decorators
                if name.starts_with("pytest.mark.") {
                    let marker = name.replace("pytest.mark.", "");
                    markers.push(marker);
                }
                // Also add non-pytest markers (like custom markers)
                else if !name.starts_with("pytest.") {
                    markers.push(name);
                }
            }
        }

        (markers, parametrize_infos)
    }

    /// Extract skip/xfail information from decorators.
    fn extract_skip_xfail_info(&self, decorators: &[ast::Expr]) -> SkipXfailInfo {
        let mut info = SkipXfailInfo::default();

        for decorator in decorators {
            if let Some(name) = self.get_decorator_name(decorator) {
                // Handle @pytest.mark.skip(reason="...")
                if name == "pytest.mark.skip" || name.starts_with("pytest.mark.skip(") {
                    info.skip = true;
                    info.skip_reason = self.extract_kwarg_string(decorator, "reason");
                }
                // Handle @pytest.mark.skipif(condition, reason="...")
                else if name == "pytest.mark.skipif" || name.starts_with("pytest.mark.skipif(") {
                    info.skipif = true;
                    info.skipif_condition = self.extract_skipif_condition(decorator);
                    info.skip_reason = self.extract_kwarg_string(decorator, "reason");
                }
                // Handle @pytest.mark.xfail(condition=..., reason=..., strict=..., raises=...)
                else if name == "pytest.mark.xfail" || name.starts_with("pytest.mark.xfail(") {
                    info.xfail = true;
                    info.xfail_condition = self.extract_xfail_condition(decorator);
                    info.xfail_reason = self.extract_kwarg_string(decorator, "reason");
                    info.xfail_strict = self.extract_kwarg_bool(decorator, "strict");
                    info.xfail_raises = self.extract_kwarg_string(decorator, "raises");
                }
            }
        }

        info
    }

    /// Extract a string keyword argument from a decorator call.
    fn extract_kwarg_string(&self, decorator: &ast::Expr, arg: &str) -> Option<String> {
        if let ast::Expr::Call(ast::ExprCall { keywords, .. }) = decorator {
            for keyword in keywords {
                if keyword.arg.as_deref() == Some(arg) {
                    if let ast::Expr::Constant(ast::ExprConstant {
                        value: ast::Constant::Str(s),
                        ..
                    }) = &keyword.value
                    {
                        return Some(s.clone());
                    }
                }
            }
        }
        None
    }

    /// Extract a boolean keyword argument from a decorator call.
    fn extract_kwarg_bool(&self, decorator: &ast::Expr, arg: &str) -> bool {
        if let ast::Expr::Call(ast::ExprCall { keywords, .. }) = decorator {
            for keyword in keywords {
                if keyword.arg.as_deref() == Some(arg) {
                    if let ast::Expr::Constant(ast::ExprConstant {
                        value: ast::Constant::Bool(b),
                        ..
                    }) = &keyword.value
                    {
                        return *b;
                    }
                }
            }
        }
        false
    }

    /// Extract skipif condition from decorator.
    fn extract_skipif_condition(&self, decorator: &ast::Expr) -> Option<String> {
        if let ast::Expr::Call(ast::ExprCall { args, keywords, .. }) = decorator {
            // Try first positional argument
            if let Some(first_arg) = args.first() {
                return Some(format!("{:?}", first_arg));
            }
            // Try condition= keyword
            for keyword in keywords {
                if keyword.arg.as_deref() == Some("condition") {
                    return Some(format!("{:?}", keyword.value));
                }
            }
        }
        None
    }

    /// Extract xfail condition from decorator.
    fn extract_xfail_condition(&self, decorator: &ast::Expr) -> Option<String> {
        if let ast::Expr::Call(ast::ExprCall { args, keywords, .. }) = decorator {
            // Try first positional argument
            if let Some(first_arg) = args.first() {
                return Some(format!("{:?}", first_arg));
            }
            // Try condition= keyword
            for keyword in keywords {
                if keyword.arg.as_deref() == Some("condition") {
                    return Some(format!("{:?}", keyword.value));
                }
            }
        }
        None
    }

    /// Extract base class names from a class definition.
    fn extract_base_class_names(&self, class_def: &ast::StmtClassDef) -> Vec<String> {
        let mut base_names = Vec::new();
        for base in &class_def.bases {
            if let ast::Expr::Name(name) = base {
                base_names.push(name.id.to_string());
            } else if let ast::Expr::Attribute(attr) = base {
                // Handle module.ClassName style
                base_names.push(attr.attr.to_string());
            }
        }
        base_names
    }

    /// Find a class definition by name in a list of statements.
    fn find_class_in_stmts<'a>(
        &self,
        stmts: &'a [ast::Stmt],
        name: &str,
    ) -> Option<&'a ast::StmtClassDef> {
        for stmt in stmts {
            if let ast::Stmt::ClassDef(class_def) = stmt {
                if class_def.name.as_str() == name {
                    return Some(class_def);
                }
            }
        }
        None
    }

    /// Collect inherited test methods from a base class.
    fn collect_inherited_tests(
        &self,
        base_class: &ast::StmtClassDef,
        derived_class_name: &str,
        derived_class_markers: &[String],
        file_path: &str,
        conftest_fixtures: &[String],
        source: &str,
        tests: &mut Vec<NativeTestNode>,
        all_stmts: &[ast::Stmt],
    ) {
        // Collect existing method names from derived class to avoid duplicates
        let existing_methods: std::collections::HashSet<String> = tests
            .iter()
            .filter(|t| t.class_name.as_deref() == Some(derived_class_name))
            .map(|t| t.name.clone())
            .collect();

        // Process test methods from base class
        let ctx = TestProcessingContext {
            file_path,
            class_name: derived_class_name, // Use derived class name for node_id
            class_markers: derived_class_markers,
            source,
        };

        for stmt in &base_class.body {
            match stmt {
                ast::Stmt::FunctionDef(func) => {
                    // Only process test methods not already defined in derived class
                    if func.name.starts_with("test_")
                        && !existing_methods.contains(&func.name.to_string())
                    {
                        self.process_function_def_with_class_markers(
                            func,
                            &ctx,
                            conftest_fixtures,
                            tests,
                        );
                    }
                }
                ast::Stmt::AsyncFunctionDef(func) => {
                    if func.name.starts_with("test_")
                        && !existing_methods.contains(&func.name.to_string())
                    {
                        self.process_async_function_def_with_class_markers(
                            func,
                            &ctx,
                            conftest_fixtures,
                            tests,
                        );
                    }
                }
                _ => {}
            }
        }

        // Recursively collect from base class's base classes
        let base_base_names = self.extract_base_class_names(base_class);
        for base_name in base_base_names {
            if let Some(base_base_class) = self.find_class_in_stmts(all_stmts, &base_name) {
                self.collect_inherited_tests(
                    base_base_class,
                    derived_class_name,
                    derived_class_markers,
                    file_path,
                    conftest_fixtures,
                    source,
                    tests,
                    all_stmts,
                );
            }
        }
    }

    /// Expand a parametrized test into multiple test nodes.
    fn expand_parametrized_tests(
        &self,
        ctx: &TestCreationContext,
        file_path: &str,
        class_name: &str,
        param_info: ParameterizedTestNode,
        tests: &mut Vec<NativeTestNode>,
    ) {
        let param_names = &param_info.param_names;
        let param_values = &param_info.param_values;

        // Detect if this parametrize has mixed types (to determine ID prefix strategy)
        // pytest uses type-specific prefix (lst, dct) when all params are same container type,
        // but uses 'value' prefix for containers when types are mixed
        let use_value_for_containers = self.has_mixed_param_types(param_values);

        // First pass: generate all base IDs to detect duplicates
        let mut base_ids: Vec<String> = Vec::with_capacity(param_values.len());
        for (idx, (values, custom_id, _)) in param_values.iter().enumerate() {
            let base_id = if let Some(id) = custom_id {
                id.clone()
            } else {
                self.generate_pytest_id_with_context(values, idx, use_value_for_containers)
            };
            base_ids.push(base_id);
        }

        // Detect duplicates and assign suffixes
        let final_ids = self.deduplicate_ids(&base_ids);

        for (idx, (values, _custom_id, variant_marks)) in param_values.iter().enumerate() {
            // Use the deduplicated ID
            let test_id = final_ids[idx].clone();

            // Build node ID with test ID suffix
            let node_id = if class_name.is_empty() {
                format!("{}::{}[{}]", file_path, ctx.fn_name, test_id)
            } else {
                format!(
                    "{}::{}::{}[{}]",
                    file_path, class_name, ctx.fn_name, test_id
                )
            };

            // Build markers - include base markers, skip/xfail, and per-variant marks
            let mut markers = ctx.markers.to_vec();
            markers.extend(variant_marks.clone());
            if ctx.skip_xfail.skip {
                markers.push("skip".to_string());
            }
            if ctx.skip_xfail.skipif {
                markers.push("skipif".to_string());
            }
            if ctx.skip_xfail.xfail {
                markers.push("xfail".to_string());
            }

            // Store parameter info for test generation
            let parameters = vec![serde_json::json!({
                "names": param_names,
                "values": values,
                "id": test_id
            })];

            tests.push(NativeTestNode {
                node_id,
                file_path: file_path.to_string(),
                name: ctx.fn_name.to_string(),
                class_name: if class_name.is_empty() {
                    None
                } else {
                    Some(class_name.to_string())
                },
                line_number: ctx.line_number,
                markers,
                is_simple: !ctx.uses_external_fixtures,
                parameters,
                // Only pre-skip unconditional @pytest.mark.skip, not @pytest.mark.skipif
                // skipif conditions can only be evaluated at runtime by pytest
                skip: ctx.skip_xfail.skip,
                skip_reason: ctx
                    .skip_xfail
                    .skip_reason
                    .clone()
                    .or(ctx.skip_xfail.skipif_condition.clone()),
                xfail: ctx.skip_xfail.xfail,
                xfail_reason: ctx.skip_xfail.xfail_reason.clone(),
                xfail_strict: ctx.skip_xfail.xfail_strict,
            });
        }
    }

    /// Expand stacked parametrized tests (multiple @pytest.mark.parametrize decorators).
    /// Generates Cartesian product of all parameter combinations.
    fn expand_stacked_parametrized_tests(
        &self,
        ctx: &TestCreationContext,
        file_path: &str,
        class_name: &str,
        param_infos: Vec<ParameterizedTestNode>,
        tests: &mut Vec<NativeTestNode>,
    ) {
        // Generate cartesian product of all parameter combinations
        let combinations = self.generate_cartesian_product(&param_infos);

        for (combo_idx, combo) in combinations.into_iter().enumerate() {
            // combo is Vec<(param_names, param_values, custom_id)>
            // Flatten into single set of names and values
            let mut all_names = Vec::new();
            let mut all_values = Vec::new();
            let mut all_marks = Vec::new(); // Combine marks from all param sets

            for (names, values, _, marks) in combo {
                all_names.extend(names);
                all_values.extend(values);
                all_marks.extend(marks);
            }

            // Generate test ID using pytest-style format
            let test_id = self.generate_pytest_id(&all_values, combo_idx);

            // Build node ID with test ID suffix
            let node_id = if class_name.is_empty() {
                format!("{}::{}[{}]", file_path, ctx.fn_name, test_id)
            } else {
                format!(
                    "{}::{}::{}[{}]",
                    file_path, class_name, ctx.fn_name, test_id
                )
            };

            // Build markers - combine base markers with param-specific marks
            let mut markers = ctx.markers.to_vec();
            markers.extend(all_marks);
            if ctx.skip_xfail.skip {
                markers.push("skip".to_string());
            }
            if ctx.skip_xfail.skipif {
                markers.push("skipif".to_string());
            }
            if ctx.skip_xfail.xfail {
                markers.push("xfail".to_string());
            }

            // Store parameter info for test generation
            let parameters = vec![serde_json::json!({
                "names": all_names,
                "values": all_values,
                "id": test_id
            })];

            tests.push(NativeTestNode {
                node_id,
                file_path: file_path.to_string(),
                name: ctx.fn_name.to_string(),
                class_name: if class_name.is_empty() {
                    None
                } else {
                    Some(class_name.to_string())
                },
                line_number: ctx.line_number,
                markers,
                is_simple: !ctx.uses_external_fixtures,
                parameters,
                // Only pre-skip unconditional @pytest.mark.skip, not @pytest.mark.skipif
                // skipif conditions can only be evaluated at runtime by pytest
                skip: ctx.skip_xfail.skip,
                skip_reason: ctx
                    .skip_xfail
                    .skip_reason
                    .clone()
                    .or(ctx.skip_xfail.skipif_condition.clone()),
                xfail: ctx.skip_xfail.xfail,
                xfail_reason: ctx.skip_xfail.xfail_reason.clone(),
                xfail_strict: ctx.skip_xfail.xfail_strict,
            });
        }
    }

    /// Generate Cartesian product of all parameter combinations from stacked parametrize.
    /// Each ParameterizedTestNode has its own param_names and param_values.
    /// Returns Vec<Vec<ParamCombination>> where each inner Vec represents
    /// one combination across all parametrize decorators.
    fn generate_cartesian_product(
        &self,
        param_infos: &[ParameterizedTestNode],
    ) -> Vec<Vec<ParamCombination>> {
        if param_infos.is_empty() {
            return Vec::new();
        }

        if param_infos.len() == 1 {
            // Single parametrize - just return each value as a separate combination
            let info = &param_infos[0];
            return info
                .param_values
                .iter()
                .map(|(values, custom_id, marks)| {
                    vec![(
                        info.param_names.clone(),
                        values.clone(),
                        custom_id.clone(),
                        marks.clone(),
                    )]
                })
                .collect();
        }

        // Multiple parametrizes - compute cartesian product
        self.compute_cartesian_recursive(param_infos, 0)
    }

    /// Recursive helper for Cartesian product computation.
    fn compute_cartesian_recursive(
        &self,
        param_infos: &[ParameterizedTestNode],
        index: usize,
    ) -> Vec<Vec<ParamCombination>> {
        if index >= param_infos.len() {
            // Base case: return empty combination
            return vec![Vec::new()];
        }

        let info = &param_infos[index];
        let remaining = self.compute_cartesian_recursive(param_infos, index + 1);

        let mut result = Vec::new();
        for (values, custom_id, marks) in info.param_values.iter() {
            for mut combo in remaining.clone() {
                combo.insert(
                    0,
                    (
                        info.param_names.clone(),
                        values.clone(),
                        custom_id.clone(),
                        marks.clone(),
                    ),
                );
                result.push(combo);
            }
        }

        result
    }

    /// Get list of fixtures used by a function.
    fn get_pytest_fixture_usage_func(
        &self,
        args: &ast::Arguments,
        conftest_fixtures: &[String],
    ) -> Vec<String> {
        let mut used = Vec::new();
        for arg in &args.args {
            let arg_name = &arg.def.arg.to_string();
            if conftest_fixtures.contains(arg_name) {
                used.push(arg_name.clone());
            }
        }
        used
    }

    /// Check if an async function uses external fixtures.
    fn check_pytest_fixture_usage_async(
        &self,
        args: &ast::Arguments,
        conftest_fixtures: &[String],
    ) -> bool {
        for arg in &args.args {
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
    use std::fs;
    use tempfile::TempDir;

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

        // Should find 4 tests (1 + 3 parametrized)
        assert_eq!(tests.len(), 4);

        // Check markers on slow test
        let slow_test = tests.iter().find(|t| t.name == "test_expensive").unwrap();
        assert!(slow_test.markers.contains(&"slow".to_string()));

        // Check parametrized tests have correct IDs
        let param_tests: Vec<&NativeTestNode> =
            tests.iter().filter(|t| t.name == "test_param").collect();
        assert_eq!(param_tests.len(), 3);

        // Check that each parametrized test has a unique ID
        let ids: Vec<&str> = param_tests
            .iter()
            .filter_map(|t| {
                // Extract ID from node_id: test_marked.py::test_param::[id]
                if let Some(start) = t.node_id.find("[") {
                    if let Some(end) = t.node_id.find("]") {
                        return Some(&t.node_id[start + 1..end]);
                    }
                }
                None
            })
            .collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_collect_async() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_async.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.asyncio
async def test_async_basic():
    assert True

async def test_async_another():
    assert True
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 2 async tests
        assert_eq!(tests.len(), 2);

        let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
        assert!(test_names.contains(&"test_async_basic"));
        assert!(test_names.contains(&"test_async_another"));
    }

    #[test]
    fn test_collect_skip_xfail() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_skip.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.skip(reason="intentional")
def test_skipped():
    pass

@pytest.mark.xfail
def test_xfailed():
    assert False
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 2 tests
        assert_eq!(tests.len(), 2);

        let skipped = tests.iter().find(|t| t.name == "test_skipped").unwrap();
        assert!(skipped.markers.contains(&"skip".to_string()));

        let xfailed = tests.iter().find(|t| t.name == "test_xfailed").unwrap();
        assert!(xfailed.markers.contains(&"xfail".to_string()));
    }

    #[test]
    fn test_collect_parametrized_class_methods() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_class_param.py");
        fs::write(
            &test_file,
            r#"
import pytest

class TestParametrized:
    @pytest.mark.parametrize("n", [1, 2, 3])
    def test_method(self, n):
        assert n > 0
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 3 parametrized method tests
        assert_eq!(tests.len(), 3);

        // Check class name is set correctly
        let param_tests: Vec<&NativeTestNode> = tests
            .iter()
            .filter(|t| t.name == "test_method" && t.class_name.is_some())
            .collect();
        assert_eq!(param_tests.len(), 3);
        assert_eq!(
            param_tests[0].class_name,
            Some("TestParametrized".to_string())
        );
    }

    #[test]
    fn test_collect_stacked_parametrize() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_stacked.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.parametrize("x", [1, 2])
@pytest.mark.parametrize("y", [10, 20])
def test_stacked_parametrize(x, y):
    """Stacked parametrize creates cartesian product."""
    assert True
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 4 tests (cartesian product of 2 x 2)
        assert_eq!(tests.len(), 4);

        // Check that all combinations are present
        let node_ids: Vec<&str> = tests
            .iter()
            .filter_map(|t| {
                if t.name == "test_stacked_parametrize" {
                    // Extract ID from node_id
                    if let Some(start) = t.node_id.find("[") {
                        if let Some(end) = t.node_id.find("]") {
                            return Some(&t.node_id[start + 1..end]);
                        }
                    }
                }
                None
            })
            .collect();

        assert_eq!(node_ids.len(), 4);
        // All combinations should be present
        assert!(node_ids.contains(&"10-1"));
        assert!(node_ids.contains(&"10-2"));
        assert!(node_ids.contains(&"20-1"));
        assert!(node_ids.contains(&"20-2"));
    }

    #[test]
    fn test_collect_class_markers() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_class_markers.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.slow
class TestMarkedClass:
    def test_one(self):
        assert True

    def test_two(self):
        assert True
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 2 tests
        assert_eq!(tests.len(), 2);

        // Both tests should inherit the "slow" marker from the class
        for test in &tests {
            assert!(
                test.markers.contains(&"slow".to_string()),
                "Test {} should have 'slow' marker from class",
                test.name
            );
        }
    }

    #[test]
    fn test_collect_skipif() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_skipif.py");
        fs::write(
            &test_file,
            r#"
import pytest
import sys

@pytest.mark.skipif(sys.version_info > (0, 0), reason="always skips")
def test_skipif():
    pass
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 1 test
        assert_eq!(tests.len(), 1);

        let test = &tests[0];
        // Should have skipif marker (not pre-skipped, since we can't evaluate the condition)
        assert!(test.markers.contains(&"skipif".to_string()));
        // skipif tests should NOT have skip=true since we can't evaluate the condition at collection time
        assert!(!test.skip);
    }

    #[test]
    fn test_collect_xfail_with_condition() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_xfail_cond.py");
        fs::write(
            &test_file,
            r#"
import pytest

@pytest.mark.xfail(condition=False, reason="condition is false")
def test_xfail_false_condition():
    assert True
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 1 test
        assert_eq!(tests.len(), 1);

        let test = &tests[0];
        // Should have xfail marker
        assert!(test.markers.contains(&"xfail".to_string()));
    }

    #[test]
    fn test_collect_param_with_per_variant_marks() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_param_marks.py");
        fs::write(
            &test_file,
            r#"
import pytest

class TestParamMarks:
    @pytest.mark.parametrize("x", [
        pytest.param(1, marks=pytest.mark.slow),
        pytest.param(2),
        pytest.param(3, marks=[pytest.mark.skip(reason="test skip")]),
    ])
    def test_param_marks(self, x):
        assert x > 0
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 3 parametrized tests (one per variant)
        assert_eq!(tests.len(), 3);

        // Check that markers are applied per-variant
        // Test 1 should have "slow" marker
        let test1 = tests.iter().find(|t| t.node_id.contains("[1]")).unwrap();
        assert!(
            test1.markers.contains(&"slow".to_string()),
            "Test 1 should have 'slow' marker"
        );

        // Test 2 should have no extra markers
        let test2 = tests.iter().find(|t| t.node_id.contains("[2]")).unwrap();
        assert!(
            !test2.markers.contains(&"slow".to_string()),
            "Test 2 should not have 'slow' marker"
        );
        assert!(
            !test2.markers.contains(&"skip".to_string()),
            "Test 2 should not have 'skip' marker"
        );

        // Test 3 should have "skip" marker (from pytest.param marks)
        let test3 = tests.iter().find(|t| t.node_id.contains("[3]")).unwrap();
        assert!(
            test3.markers.contains(&"skip".to_string()),
            "Test 3 should have 'skip' marker from pytest.param"
        );
    }

    #[test]
    fn test_line_numbers() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_lines.py");
        // Create a file with known line numbers (1-indexed)
        // Line 1: comment
        // Line 2: comment
        // Line 3: empty
        // Line 4: def test_first()
        // Line 5: body
        // Line 6: empty
        // Line 7: def test_second()
        // Line 8: body
        // Line 9: empty
        // Line 10: class
        // Line 11: def test_method
        fs::write(
            &test_file,
            r#"# Line 1 - comment
# Line 2 - comment

def test_first():
    assert True

def test_second():
    pass

class TestClass:
    def test_method(self):
        pass
"#,
        )
        .unwrap();

        let collector = NativeCollector::new(dir.path());
        let tests = collector.collect().unwrap();

        // Should find 3 tests
        assert_eq!(tests.len(), 3);

        // Verify line numbers (functions start with 'def' keyword)
        let test_first = tests.iter().find(|t| t.name == "test_first").unwrap();
        assert_eq!(test_first.line_number, 4, "test_first should be at line 4");

        let test_second = tests.iter().find(|t| t.name == "test_second").unwrap();
        assert_eq!(
            test_second.line_number, 7,
            "test_second should be at line 7"
        );

        let test_method = tests.iter().find(|t| t.name == "test_method").unwrap();
        assert_eq!(
            test_method.line_number, 11,
            "test_method should be at line 11"
        );
    }
}
