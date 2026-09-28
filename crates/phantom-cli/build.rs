use std::process::Command;

fn main() {
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
