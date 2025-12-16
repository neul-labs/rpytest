//! Output diff types for verification.

/// Kind of difference found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// Exit code differs.
    ExitCode,
    /// Test collection count differs.
    TestCount,
    /// Passed count differs.
    PassedCount,
    /// Failed count differs.
    FailedCount,
    /// Skipped count differs.
    SkippedCount,
    /// Error count differs.
    ErrorCount,
    /// Tests missing from rpytest output.
    MissingTests,
    /// Extra tests in rpytest output.
    ExtraTests,
    /// Output content differs.
    OutputContent,
    /// Timing difference (non-critical).
    Timing,
}

impl DiffKind {
    /// Whether this difference is critical for compatibility.
    pub fn is_critical(&self) -> bool {
        match self {
            DiffKind::ExitCode => true,
            DiffKind::TestCount => true,
            DiffKind::PassedCount => true,
            DiffKind::FailedCount => true,
            DiffKind::SkippedCount => true,
            DiffKind::ErrorCount => true,
            DiffKind::MissingTests => true,
            DiffKind::ExtraTests => true,
            DiffKind::OutputContent => false, // Non-critical by default
            DiffKind::Timing => false,
        }
    }

    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            DiffKind::ExitCode => "Exit code",
            DiffKind::TestCount => "Test count",
            DiffKind::PassedCount => "Passed count",
            DiffKind::FailedCount => "Failed count",
            DiffKind::SkippedCount => "Skipped count",
            DiffKind::ErrorCount => "Error count",
            DiffKind::MissingTests => "Missing tests",
            DiffKind::ExtraTests => "Extra tests",
            DiffKind::OutputContent => "Output content",
            DiffKind::Timing => "Timing",
        }
    }
}

/// A single difference between pytest and rpytest output.
#[derive(Debug, Clone)]
pub struct OutputDiff {
    /// Kind of difference.
    pub kind: DiffKind,
    /// Expected value (from pytest).
    pub expected: String,
    /// Actual value (from rpytest).
    pub actual: String,
    /// Additional context.
    pub context: String,
}

impl OutputDiff {
    /// Whether this difference is critical.
    pub fn is_critical(&self) -> bool {
        self.kind.is_critical()
    }

    /// Format as a human-readable string.
    pub fn format(&self) -> String {
        let critical = if self.is_critical() { "[CRITICAL]" } else { "[INFO]" };
        format!(
            "{} {}: expected '{}', got '{}'\n  {}",
            critical,
            self.kind.description(),
            self.expected,
            self.actual,
            self.context
        )
    }
}

impl std::fmt::Display for OutputDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_kind_critical() {
        assert!(DiffKind::ExitCode.is_critical());
        assert!(DiffKind::TestCount.is_critical());
        assert!(DiffKind::MissingTests.is_critical());
        assert!(!DiffKind::Timing.is_critical());
        assert!(!DiffKind::OutputContent.is_critical());
    }

    #[test]
    fn test_output_diff_format() {
        let diff = OutputDiff {
            kind: DiffKind::PassedCount,
            expected: "10".to_string(),
            actual: "9".to_string(),
            context: "One test fewer passed".to_string(),
        };

        let formatted = diff.format();
        assert!(formatted.contains("[CRITICAL]"));
        assert!(formatted.contains("Passed count"));
        assert!(formatted.contains("10"));
        assert!(formatted.contains("9"));
    }
}
