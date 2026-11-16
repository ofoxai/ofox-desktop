//! These tests must run on native Windows. A macOS compilation is not evidence
//! that a lowbox token, image policy, ACL, or Job is enforced.
use super::*;

fn runtime() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources")
}

#[tokio::test]
async fn document_lpac_proves_python_office_and_escape_denials() {
    let root = runtime();
    let office = root.join("engines/officecli/officecli.exe");
    probe_windows_document_engines(&root, &office)
        .await
        .expect("native Python + OfficeCLI and all LPAC negative probes must pass");
}

#[tokio::test]
async fn document_lpac_keeps_source_inputs_and_cannot_modify_runtime() {
    let root = runtime();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(task.path().join("source.pptx"), b"frozen input").unwrap();
    let script = r#"
      const fs = require('node:fs');
      const assert = require('node:assert/strict');
      assert.equal(fs.readFileSync('source.pptx', 'utf8'), 'frozen input');
      assert.throws(() => fs.writeFileSync(process.execPath, 'changed'));
      fs.writeFileSync('receipt.txt', 'checked');
    "#;
    let out = run_windows_engine_limited(
        &root.join("node/node.exe"),
        &["-e", script],
        task.path(),
        &home,
        &root,
        Duration::from_secs(20),
        4096,
    )
    .await
    .unwrap();
    assert_eq!(out.status, Some(0), "{}", out.stderr);
    assert_eq!(
        fs::read(task.path().join("receipt.txt")).unwrap(),
        b"checked"
    );
    assert_eq!(
        fs::read(task.path().join("source.pptx")).unwrap(),
        b"frozen input"
    );
}

#[tokio::test]
async fn document_lpac_timeout_prevents_late_writes_and_bounds_output() {
    let root = runtime();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let out = run_windows_engine_limited(
        &root.join("node/node.exe"),
        &["-e", "process.stdout.write('x'.repeat(100000)); setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 4000);"],
        task.path(),
        &home,
        &root,
        Duration::from_secs(1),
        1024,
    )
    .await
    .unwrap();
    assert!(out.timed_out);
    assert!(out.stdout.len() <= 1024);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!task.path().join("late").exists());
}

#[test]
fn document_output_rejects_hardlinks_and_reparse_points() {
    use std::os::windows::fs::symlink_dir;
    let root = tempfile::tempdir().unwrap();
    let task = root.path().join("task");
    let other = root.path().join("other");
    fs::create_dir(&task).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(other.join("secret"), b"outside").unwrap();
    fs::hard_link(other.join("secret"), task.join("linked")).unwrap();
    assert!(open_regular(&task, &task.join("linked"), 4096).is_err());
    // Junctions do not require developer mode or symlink privilege.
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(task.join("junction"))
        .arg(&other)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(open_regular(&task, &task.join("junction/secret"), 4096).is_err());
    if symlink_dir(&other, task.join("symlink")).is_ok() {
        assert!(open_regular(&task, &task.join("symlink/secret"), 4096).is_err());
    }
    assert_eq!(fs::read(other.join("secret")).unwrap(), b"outside");
}
