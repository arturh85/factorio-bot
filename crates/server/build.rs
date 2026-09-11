fn main() {
    // Embed git describe for build identification
    let describe = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty=-modified"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_GIT_DESCRIBE={}", describe);
    println!("cargo:rerun-if-env-changed=BUILD_GIT_DESCRIBE");
    // Also rerun when git changes
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads/master");
}
