//! Test node data structures.

use serde::{Deserialize, Serialize};

/// Unique identifier for a test node (pytest node ID format).
///
/// Examples:
/// - `test_module.py::test_function`
/// - `test_module.py::TestClass::test_method`
/// - `test_module.py::test_parametrized[param1]`
pub type TestNodeId = String;

/// The kind of test node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestNodeKind {
    /// A test function at module level.
    Function,
    /// A test method within a class.
    Method,
    /// A test class (container for methods).
    Class,
    /// A test module (container for functions/classes).
    Module,
}

impl Default for TestNodeKind {
    fn default() -> Self {
        Self::Function
    }
}

/// A single test node with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestNode {
    /// The unique node ID (pytest format).
    pub node_id: TestNodeId,

    /// The kind of node.
    pub kind: TestNodeKind,

    /// File path relative to the repo root.
    pub file_path: String,

    /// Line number where the test is defined (1-indexed).
    pub lineno: Option<u32>,

    /// The test function/method name.
    pub name: String,

    /// Parent class name (if method).
    pub class_name: Option<String>,

    /// Markers attached to this test (e.g., "slow", "skip", "xfail").
    pub markers: Vec<String>,

    /// Keywords for -k filtering (includes name, class, markers, etc.).
    pub keywords: Vec<String>,

    /// Parameter IDs if this is a parametrized test.
    pub parameters: Option<String>,

    /// Historical average duration in milliseconds.
    pub avg_duration_ms: Option<u64>,

    /// Number of times this test has been run.
    pub run_count: u32,

    /// Number of times this test has failed.
    pub fail_count: u32,

    /// Whether this test is currently marked as expected to fail.
    pub xfail: bool,

    /// Whether this test is currently skipped.
    pub skip: bool,
}

impl TestNode {
    /// Create a new test node with minimal information.
    pub fn new(node_id: impl Into<String>, file_path: impl Into<String>) -> Self {
        let node_id = node_id.into();
        let name = Self::extract_name(&node_id);
        let class_name = Self::extract_class(&node_id);
        let kind = if class_name.is_some() {
            TestNodeKind::Method
        } else {
            TestNodeKind::Function
        };

        Self {
            node_id,
            kind,
            file_path: file_path.into(),
            lineno: None,
            name,
            class_name,
            markers: Vec::new(),
            keywords: Vec::new(),
            parameters: None,
            avg_duration_ms: None,
            run_count: 0,
            fail_count: 0,
            xfail: false,
            skip: false,
        }
    }

    /// Extract the test name from a node ID.
    fn extract_name(node_id: &str) -> String {
        // Handle parametrized tests: test_foo[param] -> test_foo
        let base = if let Some(bracket_pos) = node_id.rfind('[') {
            &node_id[..bracket_pos]
        } else {
            node_id
        };

        // Get the last :: segment
        base.rsplit("::").next().unwrap_or(node_id).to_string()
    }

    /// Extract the class name from a node ID if present.
    fn extract_class(node_id: &str) -> Option<String> {
        // Handle parametrized tests first
        let base = if let Some(bracket_pos) = node_id.rfind('[') {
            &node_id[..bracket_pos]
        } else {
            node_id
        };

        let parts: Vec<&str> = base.split("::").collect();
        if parts.len() >= 3 {
            // file.py::Class::method -> Class
            Some(parts[parts.len() - 2].to_string())
        } else {
            None
        }
    }

    /// Add a marker to this test.
    pub fn add_marker(&mut self, marker: impl Into<String>) {
        let marker = marker.into();
        if !self.markers.contains(&marker) {
            // Update skip/xfail flags
            match marker.as_str() {
                "skip" | "skipif" => self.skip = true,
                "xfail" => self.xfail = true,
                _ => {}
            }
            self.markers.push(marker);
        }
    }

    /// Build the keywords list for -k filtering.
    pub fn build_keywords(&mut self) {
        self.keywords.clear();

        // Add name
        self.keywords.push(self.name.clone());

        // Add class name if present
        if let Some(ref class) = self.class_name {
            self.keywords.push(class.clone());
        }

        // Add file name without extension
        if let Some(file_stem) = self.file_path.rsplit('/').next() {
            if let Some(stem) = file_stem.strip_suffix(".py") {
                self.keywords.push(stem.to_string());
            }
        }

        // Add markers
        for marker in &self.markers {
            self.keywords.push(marker.clone());
        }

        // Add parameters if present
        if let Some(ref params) = self.parameters {
            self.keywords.push(params.clone());
        }
    }

    /// Check if this test matches a keyword expression.
    ///
    /// Simple implementation supporting:
    /// - Plain keywords (substring match)
    /// - `not keyword`
    /// - `keyword1 and keyword2`
    /// - `keyword1 or keyword2`
    pub fn matches_keyword(&self, expr: &str) -> bool {
        let expr = expr.trim();

        // Handle "not" prefix
        if let Some(rest) = expr.strip_prefix("not ") {
            return !self.matches_keyword(rest);
        }

        // Handle "and"
        if let Some((left, right)) = expr.split_once(" and ") {
            return self.matches_keyword(left) && self.matches_keyword(right);
        }

        // Handle "or"
        if let Some((left, right)) = expr.split_once(" or ") {
            return self.matches_keyword(left) || self.matches_keyword(right);
        }

        // Simple substring match
        let expr_lower = expr.to_lowercase();
        self.keywords
            .iter()
            .any(|k| k.to_lowercase().contains(&expr_lower))
            || self.node_id.to_lowercase().contains(&expr_lower)
    }

    /// Check if this test has a specific marker.
    pub fn has_marker(&self, marker: &str) -> bool {
        self.markers.iter().any(|m| m == marker)
    }

    /// Check if this test matches a marker expression.
    ///
    /// Simple implementation supporting:
    /// - Plain markers
    /// - `not marker`
    /// - `marker1 and marker2`
    /// - `marker1 or marker2`
    pub fn matches_marker(&self, expr: &str) -> bool {
        let expr = expr.trim();

        // Handle "not" prefix
        if let Some(rest) = expr.strip_prefix("not ") {
            return !self.matches_marker(rest);
        }

        // Handle "and"
        if let Some((left, right)) = expr.split_once(" and ") {
            return self.matches_marker(left) && self.matches_marker(right);
        }

        // Handle "or"
        if let Some((left, right)) = expr.split_once(" or ") {
            return self.matches_marker(left) || self.matches_marker(right);
        }

        // Simple marker match
        self.has_marker(expr)
    }

    /// Update duration statistics after a test run.
    pub fn record_duration(&mut self, duration_ms: u64) {
        self.run_count += 1;
        let current_avg = self.avg_duration_ms.unwrap_or(0);
        // Simple moving average
        self.avg_duration_ms =
            Some((current_avg * (self.run_count - 1) as u64 + duration_ms) / self.run_count as u64);
    }

    /// Record a test failure.
    pub fn record_failure(&mut self) {
        self.fail_count += 1;
    }

    /// Get the failure rate as a percentage.
    pub fn failure_rate(&self) -> f64 {
        if self.run_count == 0 {
            0.0
        } else {
            (self.fail_count as f64 / self.run_count as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_creation() {
        let node = TestNode::new(
            "test_module.py::test_function",
            "test_module.py",
        );

        assert_eq!(node.name, "test_function");
        assert_eq!(node.class_name, None);
        assert_eq!(node.kind, TestNodeKind::Function);
    }

    #[test]
    fn test_method_node() {
        let node = TestNode::new(
            "test_module.py::TestClass::test_method",
            "test_module.py",
        );

        assert_eq!(node.name, "test_method");
        assert_eq!(node.class_name, Some("TestClass".to_string()));
        assert_eq!(node.kind, TestNodeKind::Method);
    }

    #[test]
    fn test_parametrized_node() {
        let node = TestNode::new(
            "test_module.py::test_func[param1-param2]",
            "test_module.py",
        );

        assert_eq!(node.name, "test_func");
        assert_eq!(node.class_name, None);
    }

    #[test]
    fn test_keyword_matching() {
        let mut node = TestNode::new(
            "test_math.py::TestArithmetic::test_addition",
            "test_math.py",
        );
        node.add_marker("slow");
        node.build_keywords();

        assert!(node.matches_keyword("addition"));
        assert!(node.matches_keyword("Arithmetic"));
        assert!(node.matches_keyword("slow"));
        assert!(node.matches_keyword("test_math"));
        assert!(!node.matches_keyword("subtraction"));

        // Boolean expressions
        assert!(node.matches_keyword("addition and slow"));
        assert!(node.matches_keyword("addition or subtraction"));
        assert!(!node.matches_keyword("addition and fast"));
        assert!(node.matches_keyword("not subtraction"));
    }

    #[test]
    fn test_marker_matching() {
        let mut node = TestNode::new("test.py::test_func", "test.py");
        node.add_marker("slow");
        node.add_marker("integration");

        assert!(node.matches_marker("slow"));
        assert!(node.matches_marker("integration"));
        assert!(!node.matches_marker("unit"));

        assert!(node.matches_marker("slow and integration"));
        assert!(node.matches_marker("slow or unit"));
        assert!(!node.matches_marker("slow and unit"));
        assert!(node.matches_marker("not unit"));
    }

    #[test]
    fn test_duration_tracking() {
        let mut node = TestNode::new("test.py::test_func", "test.py");

        node.record_duration(100);
        assert_eq!(node.avg_duration_ms, Some(100));

        node.record_duration(200);
        assert_eq!(node.avg_duration_ms, Some(150));

        node.record_duration(300);
        assert_eq!(node.avg_duration_ms, Some(200));
    }

    #[test]
    fn test_failure_tracking() {
        let mut node = TestNode::new("test.py::test_func", "test.py");

        node.record_duration(100);
        node.record_duration(100);
        node.record_failure();
        node.record_duration(100);
        node.record_failure();

        assert_eq!(node.run_count, 3);
        assert_eq!(node.fail_count, 2);
        assert!((node.failure_rate() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_skip_xfail_markers() {
        let mut node = TestNode::new("test.py::test_func", "test.py");

        assert!(!node.skip);
        assert!(!node.xfail);

        node.add_marker("skip");
        assert!(node.skip);

        node.add_marker("xfail");
        assert!(node.xfail);
    }
}
