//! Build script for rpytest-daemon.
//!
//! Configures PyO3 conditional compilation flags when the embedded-python feature is enabled,
//! and fixes libpython dylib linking on macOS for Python installations with broken install_names
//! (e.g. mise/pyenv builds that bake in `/install/lib/` as the install_name).

use std::process::Command;

fn main() {
    // Enable PyO3 conditional compilation flags (Py_3_8, Py_3_9, etc.)
    // This allows version-specific code paths using #[cfg(Py_3_12)] etc.
    #[cfg(feature = "embedded-python")]
    pyo3_build_config::use_pyo3_cfgs();

    // Rerun build script if Python-related env vars change
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=VIRTUAL_ENV");
    println!("cargo:rerun-if-env-changed=RPYTEST_PYTHON");

    // On macOS, add an rpath to the Python library directory so the binary
    // can find libpython at runtime. This is needed because some Python
    // installations (mise, pyenv) produce a libpython with a broken
    // install_name like `/install/lib/libpython3.X.dylib`.
    #[cfg(target_os = "macos")]
    fix_macos_python_rpath();
}

#[cfg(target_os = "macos")]
fn fix_macos_python_rpath() {
    // Determine which Python to query — respect PYO3_PYTHON if set
    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".to_string());

    // Get the Python LIBDIR via sysconfig
    let output = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))",
        ])
        .output();

    let libdir = match output {
        Ok(ref out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => {
            eprintln!(
                "cargo:warning=Could not determine Python LIBDIR from `{}`. \
                 The binary may fail to find libpython at runtime.",
                python
            );
            return;
        }
    };

    if libdir.is_empty() {
        eprintln!("cargo:warning=Python LIBDIR is empty, skipping rpath fix");
        return;
    }

    // Add the Python lib directory as an rpath so the dynamic linker can find libpython
    println!("cargo:rustc-link-arg=-Wl,-rpath,{libdir}");
}
