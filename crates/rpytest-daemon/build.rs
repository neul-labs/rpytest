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

    // On macOS, fix libpython's install_name if it has a broken path.
    // Some Python installations (mise, pyenv) produce a libpython with a broken
    // install_name like `/install/lib/libpython3.X.dylib` instead of the actual path.
    // We fix this before linking so the final binary references the correct path.
    #[cfg(target_os = "macos")]
    fix_macos_libpython_install_name();
}

#[cfg(target_os = "macos")]
fn fix_macos_libpython_install_name() {
    // Determine which Python to query — respect PYO3_PYTHON if set
    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".to_string());

    // Get LIBDIR and LDLIBRARY from sysconfig
    let output = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_config_var('LIBDIR')); print(sysconfig.get_config_var('LDLIBRARY'))",
        ])
        .output();

    let (libdir, ldlibrary) = match output {
        Ok(ref out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut lines = stdout.trim().lines();
            let libdir = lines.next().unwrap_or("").to_string();
            let ldlibrary = lines.next().unwrap_or("").to_string();
            (libdir, ldlibrary)
        }
        _ => {
            eprintln!(
                "cargo:warning=Could not determine Python LIBDIR/LDLIBRARY from `{python}`. \
                 The binary may fail to find libpython at runtime."
            );
            return;
        }
    };

    if libdir.is_empty() || ldlibrary.is_empty() {
        return;
    }

    let dylib_path = format!("{libdir}/{ldlibrary}");

    // Check the current install_name of the dylib
    let otool_output = Command::new("otool")
        .args(["-D", &dylib_path])
        .output();

    let current_install_name = match otool_output {
        Ok(ref out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // otool -D output: first line is the file path, second line is the install_name
            stdout.trim().lines().nth(1).unwrap_or("").to_string()
        }
        _ => return,
    };

    // If the install_name already points to the correct path, nothing to do
    if current_install_name == dylib_path {
        return;
    }

    // The install_name is wrong — fix it using install_name_tool.
    // This modifies the dylib in-place so the linker picks up the correct path.
    eprintln!(
        "cargo:warning=Fixing libpython install_name: {current_install_name} -> {dylib_path}"
    );

    let result = Command::new("install_name_tool")
        .args(["-id", &dylib_path, &dylib_path])
        .status();

    match result {
        Ok(status) if status.success() => {
            eprintln!("cargo:warning=Successfully fixed libpython install_name");
        }
        Ok(status) => {
            eprintln!(
                "cargo:warning=install_name_tool exited with status {status}. \
                 Falling back to rpath approach."
            );
            // Fall back to rpath
            println!("cargo:rustc-link-arg=-Wl,-rpath,{libdir}");
        }
        Err(e) => {
            eprintln!(
                "cargo:warning=Failed to run install_name_tool: {e}. \
                 Falling back to rpath approach."
            );
            println!("cargo:rustc-link-arg=-Wl,-rpath,{libdir}");
        }
    }
}
