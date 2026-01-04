//! Terminal output formatting utilities.

use console::{style, Term};

/// Output handler with verbosity control.
pub struct Output {
    term: Term,
    verbose: u8,
    quiet: u8,
}

impl Output {
    /// Create a new output handler.
    pub fn new(verbose: u8, quiet: u8) -> Self {
        Self {
            term: Term::stderr(),
            verbose,
            quiet,
        }
    }

    /// Get the effective verbosity level.
    /// Positive = verbose, negative = quiet, 0 = normal.
    fn verbosity(&self) -> i8 {
        self.verbose as i8 - self.quiet as i8
    }

    /// Print a header line.
    pub fn header(&self, msg: &str) {
        if self.verbosity() >= -1 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("===").cyan().bold(),
                style(msg).bold()
            ));
        }
    }

    /// Print an info message.
    pub fn info(&self, msg: &str) {
        if self.verbosity() >= 0 {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print a detail message (verbose only).
    pub fn detail(&self, msg: &str) {
        if self.verbosity() >= 1 {
            let _ = self.term.write_line(&style(msg).dim().to_string());
        }
    }

    /// Print a debug message (very verbose only).
    pub fn debug(&self, msg: &str) {
        if self.verbosity() >= 2 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("[DEBUG]").dim(),
                style(msg).dim()
            ));
        }
    }

    /// Print a success message.
    pub fn success(&self, msg: &str) {
        if self.verbosity() >= -1 {
            let _ = self
                .term
                .write_line(&format!("{} {}", style("✓").green(), msg));
        }
    }

    /// Print a warning message.
    pub fn warn(&self, msg: &str) {
        let _ = self
            .term
            .write_line(&format!("{} {}", style("⚠").yellow(), msg));
    }

    /// Print an error message.
    pub fn error(&self, msg: &str) {
        let _ = self
            .term
            .write_line(&format!("{} {}", style("✗").red(), style(msg).red()));
    }

    /// Print a test passed indicator.
    pub fn test_passed(&self, node_id: &str) {
        if self.verbosity() >= 1 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("PASSED").green(),
                node_id
            ));
        } else if self.verbosity() >= 0 {
            let _ = self.term.write_str(&style(".").green().to_string());
        }
    }

    /// Print a test failed indicator.
    pub fn test_failed(&self, node_id: &str) {
        if self.verbosity() >= 1 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("FAILED").red(),
                node_id
            ));
        } else if self.verbosity() >= 0 {
            let _ = self.term.write_str(&style("F").red().to_string());
        }
    }

    /// Print a test skipped indicator.
    pub fn test_skipped(&self, node_id: &str) {
        if self.verbosity() >= 1 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("SKIPPED").yellow(),
                node_id
            ));
        } else if self.verbosity() >= 0 {
            let _ = self.term.write_str(&style("s").yellow().to_string());
        }
    }

    /// Print a test error indicator.
    pub fn test_error(&self, node_id: &str) {
        if self.verbosity() >= 1 {
            let _ = self.term.write_line(&format!(
                "{} {}",
                style("ERROR").red().bold(),
                node_id
            ));
        } else if self.verbosity() >= 0 {
            let _ = self.term.write_str(&style("E").red().bold().to_string());
        }
    }

    /// Print a newline (used after dot-style output).
    pub fn newline(&self) {
        if self.verbosity() >= 0 {
            let _ = self.term.write_line("");
        }
    }

    /// Print summary statistics.
    pub fn summary(&self, passed: usize, failed: usize, skipped: usize, errors: usize, duration_secs: f64) {
        if self.verbosity() >= -1 {
            let _ = self.term.write_line("");

            let parts: Vec<String> = [
                (passed, "passed", "green"),
                (failed, "failed", "red"),
                (skipped, "skipped", "yellow"),
                (errors, "errors", "red"),
            ]
            .iter()
            .filter(|(count, _, _)| *count > 0)
            .map(|(count, label, color)| {
                let styled = match *color {
                    "green" => style(format!("{} {}", count, label)).green(),
                    "red" => style(format!("{} {}", count, label)).red(),
                    "yellow" => style(format!("{} {}", count, label)).yellow(),
                    _ => style(format!("{} {}", count, label)),
                };
                styled.to_string()
            })
            .collect();

            let summary = if parts.is_empty() {
                "no tests ran".to_string()
            } else {
                parts.join(", ")
            };

            let _ = self.term.write_line(&format!(
                "{} {} in {:.2}s {}",
                style("===").cyan().bold(),
                summary,
                duration_secs,
                style("===").cyan().bold()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verbosity_normal() {
        let output = Output::new(0, 0);
        assert_eq!(output.verbosity(), 0);
    }

    #[test]
    fn test_verbosity_verbose() {
        let output = Output::new(1, 0);
        assert_eq!(output.verbosity(), 1);

        let output = Output::new(2, 0);
        assert_eq!(output.verbosity(), 2);
    }

    #[test]
    fn test_verbosity_quiet() {
        let output = Output::new(0, 1);
        assert_eq!(output.verbosity(), -1);

        let output = Output::new(0, 2);
        assert_eq!(output.verbosity(), -2);
    }

    #[test]
    fn test_verbosity_mixed() {
        // verbose and quiet cancel out
        let output = Output::new(2, 1);
        assert_eq!(output.verbosity(), 1);

        let output = Output::new(1, 2);
        assert_eq!(output.verbosity(), -1);

        let output = Output::new(3, 3);
        assert_eq!(output.verbosity(), 0);
    }
}
