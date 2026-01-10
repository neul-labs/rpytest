//! Build script for rpytest-daemon.
//!
//! Configures PyO3 conditional compilation flags when the embedded-python feature is enabled.

fn main() {
    // Enable PyO3 conditional compilation flags (Py_3_8, Py_3_9, etc.)
    // This allows version-specific code paths using #[cfg(Py_3_12)] etc.
    #[cfg(feature = "embedded-python")]
    pyo3_build_config::use_pyo3_cfgs();

    // Rerun build script if Python-related env vars change
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=VIRTUAL_ENV");
    println!("cargo:rerun-if-env-changed=RPYTEST_PYTHON");
}
