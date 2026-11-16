//! These tests must run on native Windows. A macOS compilation is not evidence
//! that a lowbox token, image policy, ACL, or Job is enforced.
use super::*;

fn runtime() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources")
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_proves_python_office_and_escape_denials() {
    let root = runtime();
    let office = root.join("engines/officecli/officecli.exe");
    probe_windows_document_engines(&root, &office)
        .await
        .expect("native Python + OfficeCLI and all LPAC negative probes must pass");
}

#[tokio::test]
#[serial_test::serial]
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
#[serial_test::serial]
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
    publish_output(root.path(), Path::new("a"), b"short name").unwrap();
    assert_eq!(fs::read(root.path().join("a")).unwrap(), b"short name");
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
    let mut workspace = task_directory(&user, profile.sid.0).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(workspace.0.join("outside-junction"))
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success());
    let task_path = workspace.0.clone();
    workspace.dispose().unwrap();
    profile.dispose().unwrap();
    assert!(!task_path.exists());
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"preserved"
    );
}

#[test]
fn document_cleanup_failure_is_reported_and_can_be_retried() {
    let capability = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let mut workspace = task_directory(&user, profile.sid.0).unwrap();
    fs::write(workspace.0.join("locked"), b"owned test data").unwrap();
    let locked = open_regular(&workspace.0, &workspace.0.join("locked"), 1024).unwrap();
    assert_eq!(
        workspace.dispose().unwrap_err(),
        "assistant_document_sandbox_cleanup_failed"
    );
    drop(locked);
    workspace.dispose().unwrap();
    profile.dispose().unwrap();
    assert!(!workspace.0.exists());
}

#[tokio::test]
#[serial_test::serial]
async fn failed_profile_cleanup_revokes_admission_before_another_engine_can_start() {
    // A synthetic executable must never be launched. Its deliberately seeded
    // proof lets this exercise cleanup revocation without a paid/model run.
    let public = tempfile::tempdir().unwrap();
    fs::create_dir(public.path().join("node")).unwrap();
    let node = public.path().join("node/node.exe");
    fs::write(&node, b"synthetic executable; must never launch").unwrap();
    let capability = RuntimeCapability::new().unwrap();
    let cached = cache_runtime(public.path(), &capability, &current_user().unwrap()).unwrap();
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(cached.source, cached.digest);
    assert!(windows_document_engine_proven(public.path()));
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let folder = profile.directory.as_ref().unwrap().clone();
    let marker = folder.join("held-open-test-file");
    fs::write(&marker, b"synthetic lock").unwrap();
    let locked = open_regular(&folder, &marker, 1024).unwrap();
    assert_eq!(
        dispose_profile(&mut profile).unwrap_err(),
        "assistant_document_sandbox_cleanup_failed"
    );
    assert!(!windows_document_engine_proven(public.path()));
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    assert_eq!(
        run_windows_engine_limited(
            &node,
            &[],
            task.path(),
            &home,
            public.path(),
            Duration::from_secs(1),
            1024
        )
        .await
        .unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    drop(locked);
    dispose_profile(&mut profile).unwrap();
}

#[tokio::test]
#[serial_test::serial]
async fn document_child_crash_cleans_task_and_profile_before_receipt() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let out = run_windows_engine_limited(&root.join("node/node.exe"),
        &["-e", "setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 2000); process.abort();"],
        task.path(), &home, &root, Duration::from_secs(15), 4096).await.unwrap();
    assert!(out.status.is_some_and(|status| status != 0));
    let receipts = windows_document_sandbox_receipts(sequence);
    let receipt = receipts
        .iter()
        .find(|row| row["trustedProbe"] == false)
        .unwrap();
    assert_eq!(receipt["cleanupComplete"], true);
    assert!(!std::env::temp_dir()
        .join(receipt["taskId"].as_str().unwrap())
        .exists());
    assert!(!task.path().join("late").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_an_active_document_job_cleans_before_receipt_and_no_late_write() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let canonical = task.path().canonicalize().unwrap();
    let original = task.path().to_path_buf();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let running = tokio::spawn(async move {
        run_windows_engine_limited(&root.join("node/node.exe"),
            &["-e", "setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 4000); setInterval(() => {}, 1000);"],
            &original, &home, &root, Duration::from_secs(30), 4096).await
    });
    let sandbox = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let active = ACTIVE_TASKS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .get(&canonical)
                .cloned();
            if let Some(path) = active {
                break path;
            }
            assert!(
                !running.is_finished(),
                "the controlled job failed before starting"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    running.abort();
    let _ = running.await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .contains_key(&canonical)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!sandbox.exists());
    assert!(!task.path().join("late").exists());
    let receipts = windows_document_sandbox_receipts(sequence);
    let row = receipts
        .iter()
        .find(|row| row["taskId"].as_str() == sandbox.file_name().and_then(OsStr::to_str))
        .unwrap();
    assert_eq!(row["cancelled"], true);
    assert_eq!(row["cleanupComplete"], true);
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_after_exit_stops_all_copyback_publication() {
    cancel_at_copyback(CopybackPoint::Drained).await;
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_between_copyback_files_stops_remaining_publication() {
    cancel_at_copyback(CopybackPoint::Published(1)).await;
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_after_staging_fsync_does_not_publish_the_staged_file() {
    cancel_at_copyback(CopybackPoint::BeforeRename).await;
}

async fn cancel_at_copyback(point: CopybackPoint) {
    let regression = match point {
        CopybackPoint::Drained => "COPYBACK_CANCEL_REGRESSION_AFTER_EXIT",
        CopybackPoint::Published(_) => "COPYBACK_CANCEL_REGRESSION_BETWEEN_FILES",
        CopybackPoint::BeforeRename => "COPYBACK_CANCEL_REGRESSION_BEFORE_RENAME",
    };
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let canonical = task.path().canonicalize().unwrap();
    let (reached, started) = std::sync::mpsc::sync_channel(1);
    let (resume, resumed) = std::sync::mpsc::sync_channel(1);
    COPYBACK_GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(
            canonical.clone(),
            Arc::new(CopybackGate {
                point,
                reached,
                resume: Mutex::new(resumed),
            }),
        );
    let original = task.path().to_path_buf();
    let runtime = root.clone();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let running = tokio::spawn(async move {
        run_windows_engine_limited(&runtime.join("node/node.exe"),
                &["-e", "const f=require('node:fs'); f.writeFileSync('a.txt','first'); f.writeFileSync('b.txt','second');"],
                &original, &home, &runtime, Duration::from_secs(30), 4096).await
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(Duration::from_secs(60)))
        .await
        .unwrap()
        .expect("controlled successful child must reach the copyback handshake");
    let sandbox = ACTIVE_TASKS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&canonical)
        .cloned()
        .unwrap();
    running.abort();
    let _ = running.await;
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .contains_key(&canonical)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    COPYBACK_GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(&canonical);
    assert_eq!(
        task.path().join("a.txt").exists(),
        point == CopybackPoint::Published(1),
        "{regression}: first output was published after cancellation"
    );
    assert!(
        !task.path().join("b.txt").exists(),
        "{regression}: remaining output was published after cancellation"
    );
    assert!(fs::read_dir(task.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".ofox-publish-")));
    assert!(!sandbox.exists());
    let receipts = windows_document_sandbox_receipts(sequence);
    let row = receipts
        .iter()
        .find(|row| row["taskId"].as_str() == sandbox.file_name().and_then(OsStr::to_str))
        .unwrap();
    assert_eq!(
        row["cancelled"], true,
        "{regression}: cancellation was lost"
    );
    assert_eq!(
        row["status"],
        serde_json::Value::Null,
        "{regression}: cancelled copyback reported successful completion"
    );
    assert_eq!(row["cleanupComplete"], true);
}

#[tokio::test]
#[serial_test::serial]
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
