fn main() {
    emit_source_identity();
    tauri_build::build();

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    #[cfg(target_os = "windows")]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }
}

// Bind test-only native-writer receipts to the compiled checkout, not a caller's
// claimed SHA. No credential or Git file content is emitted into build logs.
fn emit_source_identity() {
    use std::{path::PathBuf, process::Command};
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .parent()
        .expect("repository root")
        .to_path_buf();
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let sha = sha.trim();
    let valid = sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit());
    let clean = git(&["status", "--porcelain", "--untracked-files=normal"])
        .is_some_and(|status| status.lines().all(|line| line == "?? HANDOFF-codex.md"));
    println!(
        "cargo:rustc-env=OFOX_BUILD_SOURCE_SHA={}",
        if valid { sha } else { "unknown" }
    );
    println!("cargo:rustc-env=OFOX_BUILD_SOURCE_CLEAN={}", valid && clean);
    // Resolve Git's own paths so linked worktrees use their real HEAD, index,
    // and common ref directory. Ref contents change when the final SHA changes.
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            let path = PathBuf::from(path.trim());
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", reference.trim()]) {
            let path = PathBuf::from(path.trim());
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    if let Some(files) = git(&["ls-files", "-z"]) {
        for file in files
            .split('\0')
            .filter(|file| !file.is_empty() && !file.contains(['\r', '\n']))
        {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
    }
}
