// SPDX-License-Identifier: GPL-3.0-or-later
// Shared by the build scripts of the boot stage and the application (included with `include!`).
// Sets G7250_REV to the short source revision ("<git id>" or "<git id>-dirty"), or to the value
// of the G7250_REV environment variable if set, and links the chosen memory script.
fn g7250_rev() -> String {
    if let Ok(v) = std::env::var("G7250_REV") {
        return v;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let fw = std::path::Path::new(&dir).join("../..");
    let id = std::process::Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .current_dir(&fw)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "--", "."])
        .current_dir(&fw)
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if dirty { format!("{id}-dirty") } else { id }
}

fn link_memory_x(script: &str) {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    std::fs::copy(std::path::Path::new(&dir).join(script), out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed={script}");
    println!("cargo:rerun-if-env-changed=G7250_REV");
    println!("cargo:rustc-env=G7250_REV={}", g7250_rev());
}
