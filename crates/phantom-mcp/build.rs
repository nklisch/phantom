// Embed an rpath to the vendored libghostty-vt dylib so the phantom-mcp
// binary can locate it at runtime. Mirrors the logic in phantom-daemon's
// build.rs — both binaries link transitively against libghostty-vt.

use std::path::Path;
use std::process::Command;

fn main() {
    embed_git_commit();
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GHOSTTY");

    // Nothing to link against unless the ghostty backend is in the build.
    if std::env::var_os("CARGO_FEATURE_GHOSTTY").is_none() {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);

    // OUT_DIR layout: target/{profile}/build/phantom-mcp-HASH/out
    // We want to find target/{profile}/build/libghostty-vt-sys-HASH/out/ghostty-install/lib
    if let Some(build_dir) = out_path.parent().and_then(|p| p.parent())
        && let Ok(entries) = std::fs::read_dir(build_dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("libghostty-vt-sys-") {
                let lib_dir = entry.path().join("out/ghostty-install/lib");
                if lib_dir.exists() {
                    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
                    return;
                }
            }
        }
    }
}

fn embed_git_commit() {
    println!("cargo:rerun-if-env-changed=PHANTOM_BUILD_COMMIT");
    let commit = std::env::var("PHANTOM_BUILD_COMMIT").unwrap_or_else(|_| {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git is required to embed the Phantom build commit");
        assert!(output.status.success(), "failed to read Phantom git commit");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    });
    println!("cargo:rustc-env=PHANTOM_BUILD_COMMIT={commit}");

    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--git-path", "HEAD"])
        .output()
    {
        let path = String::from_utf8_lossy(&output.stdout);
        println!("cargo:rerun-if-changed={}", path.trim());
    }
    if let Ok(output) = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        && output.status.success()
    {
        let reference = String::from_utf8_lossy(&output.stdout);
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--git-path", reference.trim()])
            .output()
        {
            let path = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rerun-if-changed={}", path.trim());
        }
    }
}
