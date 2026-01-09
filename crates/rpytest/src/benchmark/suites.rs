//! Test suite generation for benchmarks.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Predefined suite sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteSize {
    /// Tiny suite (5-10 tests) - measures startup overhead.
    Tiny,
    /// Small suite (20-50 tests) - unit test repos.
    Small,
    /// Medium suite (100-200 tests) - typical project.
    Medium,
    /// Large suite (500-1000 tests) - large project.
    Large,
    /// Custom size.
    Custom(usize),
}

impl SuiteSize {
    /// Get the target test count.
    pub fn test_count(&self) -> usize {
        match self {
            SuiteSize::Tiny => 10,
            SuiteSize::Small => 50,
            SuiteSize::Medium => 200,
            SuiteSize::Large => 1000,
            SuiteSize::Custom(n) => *n,
        }
    }

    /// Get a descriptive name.
    pub fn name(&self) -> &'static str {
        match self {
            SuiteSize::Tiny => "tiny",
            SuiteSize::Small => "small",
            SuiteSize::Medium => "medium",
            SuiteSize::Large => "large",
            SuiteSize::Custom(_) => "custom",
        }
    }
}

/// Test characteristics for suite generation.
#[derive(Debug, Clone)]
pub struct TestSuite {
    /// Suite name.
    pub name: String,
    /// Root directory.
    pub root: PathBuf,
    /// Number of tests.
    pub test_count: usize,
    /// Number of test files.
    pub file_count: usize,
    /// Average test duration in milliseconds.
    pub avg_duration_ms: u64,
    /// Whether tests use fixtures.
    pub uses_fixtures: bool,
    /// Whether tests use parametrize.
    pub uses_parametrize: bool,
    /// Whether tests have markers.
    pub uses_markers: bool,
}

/// Generator for synthetic test suites.
pub struct SuiteGenerator {
    /// Base directory for generated suites.
    pub base_dir: PathBuf,
}

impl SuiteGenerator {
    /// Create a new generator.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Generate a test suite.
    pub fn generate(&self, size: SuiteSize, options: &SuiteOptions) -> Result<TestSuite> {
        let test_count = size.test_count();
        let suite_name = format!("bench_{}", size.name());
        let suite_dir = self.base_dir.join(&suite_name);

        // Clean and create directory
        if suite_dir.exists() {
            fs::remove_dir_all(&suite_dir)?;
        }
        fs::create_dir_all(&suite_dir)?;

        // Generate conftest.py if needed
        if options.uses_fixtures {
            self.generate_conftest(&suite_dir, options)?;
        }

        // Calculate file distribution
        let tests_per_file = options.tests_per_file.unwrap_or(20);
        let file_count = test_count.div_ceil(tests_per_file);

        // Generate test files
        let mut tests_generated = 0;
        for i in 0..file_count {
            let tests_in_file = std::cmp::min(tests_per_file, test_count - tests_generated);
            self.generate_test_file(&suite_dir, i, tests_in_file, options)?;
            tests_generated += tests_in_file;
        }

        // Create pytest.ini
        self.generate_pytest_ini(&suite_dir)?;

        Ok(TestSuite {
            name: suite_name,
            root: suite_dir,
            test_count,
            file_count,
            avg_duration_ms: options.avg_duration_ms,
            uses_fixtures: options.uses_fixtures,
            uses_parametrize: options.uses_parametrize,
            uses_markers: options.uses_markers,
        })
    }

    fn generate_conftest(&self, dir: &Path, options: &SuiteOptions) -> Result<()> {
        let mut content = String::from("import pytest\nimport time\n\n");

        // Simple fixture
        content.push_str(
            "@pytest.fixture\n\
             def simple_fixture():\n\
             \"\"\"A simple fixture.\"\"\"\n    \
             return 42\n\n",
        );

        // Module-scoped fixture
        content.push_str(
            "@pytest.fixture(scope=\"module\")\n\
             def module_fixture():\n\
             \"\"\"Module-scoped fixture.\"\"\"\n    \
             return {\"data\": \"test\"}\n\n",
        );

        // Session fixture (if fixture reuse testing)
        if options.uses_session_fixtures {
            content.push_str(
                "@pytest.fixture(scope=\"session\")\n\
                 def session_fixture():\n\
                 \"\"\"Session-scoped fixture.\"\"\"\n    \
                 time.sleep(0.01)  # Simulate setup\n    \
                 return \"session_data\"\n\n",
            );
        }

        let path = dir.join("conftest.py");
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;

        Ok(())
    }

    fn generate_test_file(
        &self,
        dir: &Path,
        file_idx: usize,
        test_count: usize,
        options: &SuiteOptions,
    ) -> Result<()> {
        let mut content = String::from("import pytest\nimport time\n\n");

        for i in 0..test_count {
            let test_idx = file_idx * 100 + i;
            content.push_str(&self.generate_test_function(test_idx, options));
            content.push('\n');
        }

        let filename = format!("test_suite_{:03}.py", file_idx);
        let path = dir.join(filename);
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;

        Ok(())
    }

    fn generate_test_function(&self, idx: usize, options: &SuiteOptions) -> String {
        let mut content = String::new();

        // Add markers
        if options.uses_markers && idx % 3 == 0 {
            content.push_str("@pytest.mark.slow\n");
        }
        if options.uses_markers && idx % 5 == 0 {
            content.push_str("@pytest.mark.integration\n");
        }

        // Add parametrize
        if options.uses_parametrize && idx % 4 == 0 {
            content.push_str("@pytest.mark.parametrize(\"value\", [1, 2, 3])\n");
        }

        // Function signature
        let params = if options.uses_fixtures {
            if idx % 2 == 0 {
                "simple_fixture"
            } else {
                ""
            }
        } else {
            ""
        };

        let param_str = if options.uses_parametrize && idx % 4 == 0 {
            if params.is_empty() {
                "value"
            } else {
                "simple_fixture, value"
            }
        } else {
            params
        };

        content.push_str(&format!("def test_func_{:04}({}):\n", idx, param_str));

        // Test body
        if options.avg_duration_ms > 0 {
            let sleep_time = (options.avg_duration_ms as f64) / 1000.0;
            content.push_str(&format!("    time.sleep({:.4})\n", sleep_time));
        }

        // Assertion
        if options.uses_parametrize && idx % 4 == 0 {
            content.push_str("    assert value in [1, 2, 3]\n");
        } else if options.uses_fixtures && idx % 2 == 0 {
            content.push_str("    assert simple_fixture == 42\n");
        } else {
            content.push_str(&format!("    assert {} == {}\n", idx, idx));
        }

        content
    }

    fn generate_pytest_ini(&self, dir: &Path) -> Result<()> {
        let content = "[pytest]\n\
            testpaths = .\n\
            python_files = test_*.py\n\
            python_functions = test_*\n\
            markers =\n    \
            slow: marks tests as slow\n    \
            integration: marks integration tests\n";

        let path = dir.join("pytest.ini");
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;

        Ok(())
    }

    /// Clean up generated suites.
    pub fn cleanup(&self) -> Result<()> {
        if self.base_dir.exists() {
            fs::remove_dir_all(&self.base_dir)?;
        }
        Ok(())
    }
}

/// Options for suite generation.
#[derive(Debug, Clone)]
pub struct SuiteOptions {
    /// Average test duration in milliseconds.
    pub avg_duration_ms: u64,
    /// Tests per file.
    pub tests_per_file: Option<usize>,
    /// Whether to use fixtures.
    pub uses_fixtures: bool,
    /// Whether to use parametrize.
    pub uses_parametrize: bool,
    /// Whether to use markers.
    pub uses_markers: bool,
    /// Whether to use session fixtures.
    pub uses_session_fixtures: bool,
}

impl Default for SuiteOptions {
    fn default() -> Self {
        Self {
            avg_duration_ms: 0,
            tests_per_file: Some(20),
            uses_fixtures: false,
            uses_parametrize: false,
            uses_markers: false,
            uses_session_fixtures: false,
        }
    }
}

impl SuiteOptions {
    /// Create options for overhead benchmarking (fast tests).
    pub fn overhead() -> Self {
        Self {
            avg_duration_ms: 0,
            tests_per_file: Some(20),
            uses_fixtures: false,
            uses_parametrize: false,
            uses_markers: false,
            uses_session_fixtures: false,
        }
    }

    /// Create options for realistic benchmarking.
    pub fn realistic() -> Self {
        Self {
            avg_duration_ms: 5,
            tests_per_file: Some(20),
            uses_fixtures: true,
            uses_parametrize: true,
            uses_markers: true,
            uses_session_fixtures: false,
        }
    }

    /// Create options for IO-heavy benchmarking.
    pub fn io_heavy() -> Self {
        Self {
            avg_duration_ms: 50,
            tests_per_file: Some(10),
            uses_fixtures: true,
            uses_parametrize: false,
            uses_markers: false,
            uses_session_fixtures: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_suite_size() {
        assert_eq!(SuiteSize::Tiny.test_count(), 10);
        assert_eq!(SuiteSize::Medium.test_count(), 200);
        assert_eq!(SuiteSize::Custom(42).test_count(), 42);
    }

    #[test]
    fn test_generate_suite() {
        let tmp = tempdir().unwrap();
        let generator = SuiteGenerator::new(tmp.path());

        let suite = generator
            .generate(SuiteSize::Tiny, &SuiteOptions::default())
            .unwrap();

        assert_eq!(suite.test_count, 10);
        assert!(suite.root.exists());
        assert!(suite.root.join("pytest.ini").exists());
    }

    #[test]
    fn test_generate_with_fixtures() {
        let tmp = tempdir().unwrap();
        let generator = SuiteGenerator::new(tmp.path());

        let suite = generator
            .generate(SuiteSize::Tiny, &SuiteOptions::realistic())
            .unwrap();

        assert!(suite.root.join("conftest.py").exists());
        assert!(suite.uses_fixtures);
    }
}
