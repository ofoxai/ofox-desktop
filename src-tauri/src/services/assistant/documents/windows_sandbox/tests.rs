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
    ensure_windows_document_engine_ready(&root).await.unwrap();
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
    ensure_windows_document_engine_ready(&root).await.unwrap();
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
    // The apparent root itself can be reached through an earlier junction.
    // Checking only the final parent directory would accept this case.
    fs::create_dir(other.join("nested")).unwrap();
    fs::write(other.join("nested/secret"), b"outside nested").unwrap();
    assert!(open_regular(
        &task.join("junction/nested"),
        &task.join("junction/nested/secret"),
        4096
    )
    .is_err());
    assert!(publish_output(&task, Path::new("junction/new.txt"), b"must not escape").is_err());
    assert!(!other.join("new.txt").exists());
    if symlink_dir(&other, task.join("symlink")).is_ok() {
        assert!(open_regular(&task, &task.join("symlink/secret"), 4096).is_err());
    }
    assert_eq!(fs::read(other.join("secret")).unwrap(), b"outside");
}

#[test]
fn document_output_publishes_new_and_existing_files_by_parent_handle() {
    let root = tempfile::tempdir().unwrap();
    publish_output(root.path(), Path::new("nested/new.txt"), b"first").unwrap();
    assert_eq!(
        fs::read(root.path().join("nested/new.txt")).unwrap(),
        b"first"
    );
    publish_output(root.path(), Path::new("nested/new.txt"), b"second").unwrap();
    assert_eq!(
        fs::read(root.path().join("nested/new.txt")).unwrap(),
        b"second"
    );
}

#[test]
fn document_workspace_cleanup_unlinks_junction_without_reading_target() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"preserved").unwrap();
    let cap = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let mut profile = Profile::new(&cap).unwrap();
    profile.make_low_profile().unwrap();
    let workspace = task_directory(&user, profile.sid.0).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(workspace.0.join("outside-junction"))
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success());
    let task_path = workspace.0.clone();
    drop(workspace);
    drop(profile);
    assert!(!task_path.exists());
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"preserved"
    );
}

#[tokio::test]
async fn public_runner_refuses_unproven_or_changed_runtime_before_launch() {
    let public = tempfile::tempdir().unwrap();
    fs::create_dir(public.path().join("node")).unwrap();
    let node = public.path().join("node/node.exe");
    fs::write(&node, b"synthetic executable; must never launch").unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let run = || {
        run_windows_engine_limited(
            &node,
            &[],
            task.path(),
            &home,
            public.path(),
            Duration::from_secs(1),
            1024,
        )
    };
    assert_eq!(
        run().await.unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    let cap = RuntimeCapability::new().unwrap();
    let cached = cache_runtime(public.path(), &cap, &current_user().unwrap()).unwrap();
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(cached.source.clone(), cached.digest);
    fs::write(
        &node,
        b"changed synthetic executable; must still never launch",
    )
    .unwrap();
    assert_eq!(
        run().await.unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(&cached.source);
}

#[test]
fn warm_public_cache_rejects_changed_cached_bytes() {
    let public = tempfile::tempdir().unwrap();
    fs::write(
        public.path().join("resource"),
        format!("original public asset {}", uuid::Uuid::new_v4()),
    )
    .unwrap();
    let capability = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let cached = cache_runtime(public.path(), &capability, &user).unwrap();
    fs::write(cached.root.join("resource"), b"corrupt cached public asset").unwrap();
    assert!(cache_runtime(public.path(), &capability, &user).is_err());
    // Clean only this test's known complete public cache, not another task.
    fs::remove_dir_all(&cached.root).unwrap();
}
