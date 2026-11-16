use super::super::tools::scope::resolve as resolve_local_test_path;

struct LocalMutationFixture {
    path: PathBuf,
    change: bool,
    fail_after_write: Option<&'static str>,
    written: tokio::sync::Notify,
}

struct LocalKeyBarrier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial]
async fn home_retarget_during_journal_key_wait_is_refused_before_mutation() {
    let _sandbox = crate::test_sandbox::sandbox();
    let (_tmp, env, host) = fixture_with_env();
    let original = env.config.join("original-home");
    let other = env.config.join("other-home");
    for home in [&original, &other] {
        std::fs::create_dir_all(home.join("config")).unwrap();
        std::fs::write(home.join("config/target.json"), b"unchanged").unwrap();
    }
    let alias = env.config.join("changing-home");
    std::os::unix::fs::symlink(&original, &alias).unwrap();
    *env.local_home.lock().unwrap() = Some(alias.clone());
    let barrier = Arc::new(LocalKeyBarrier {
        entered: Default::default(),
        release: Default::default(),
    });
    *env.local_key_barrier.lock().unwrap() = Some(barrier.clone());
    *env.local_mutation_fixture.lock().unwrap() = Some(Arc::new(LocalMutationFixture {
        path: alias.join("config/target.json"),
        change: true,
        fail_after_write: Some("fixture_mutator_called"),
        written: Default::default(),
    }));
    let (sid, first) = local_run(&host);
    host.release_run(&first);
    host.set_permission(super::local::PermissionRequest {
        session_id: sid.clone(),
        mode: super::super::tools::policy::PermissionMode::FullAuto,
        confirmed_full_access: true,
        expected_owner: identity().owner,
        expected_generation: host.generation(),
    })
    .await
    .unwrap();
    let mut run = control(&sid);
    let state = Arc::get_mut(&mut run).unwrap();
    state.local_enabled = true;
    state.local_home = Some(resolve_local_test_path(&original).unwrap());
    host.register_run(run.clone(), host.generation(), || Ok(()))
        .unwrap();
    let call = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"key-wait-retarget","name":"rebind_tool","args":{"tool":"codex"}});
    let work = tokio::spawn({
        let host = host.clone();
        let run = run.clone();
        async move { host.dispatch_tool(&run, &call).await }
    });
    tokio::time::timeout(Duration::from_secs(5), barrier.entered.notified())
        .await
        .unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&other, &alias).unwrap();
    barrier.release.notify_one();
    assert_eq!(
        work.await.unwrap().unwrap_err(),
        "assistant_local_scope_changed"
    );
    for home in [&original, &other] {
        assert_eq!(
            std::fs::read(home.join("config/target.json")).unwrap(),
            b"unchanged"
        );
    }
    assert!(host.local_task_state(&identity().owner, &sid).is_null());
    host.release_run(&run);
}

#[tokio::test]
#[serial_test::serial]
async fn interrupted_journaled_mutation_keeps_verification_debt_across_run_result() {
    let _sandbox = crate::test_sandbox::sandbox();
    for cancel in [true, false] {
        let (_tmp, env, host) = fixture_with_env();
        // Admission uses scope::resolve, not std canonicalize's Windows
        // verbatim spelling, which generic tool paths intentionally reject.
        let admitted_home = resolve_local_test_path(&env.config).unwrap();
        let path = env.config.join("config/target.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"original").unwrap();
        let fixture = Arc::new(LocalMutationFixture {
            path: path.clone(),
            change: true,
            fail_after_write: None,
            written: Default::default(),
        });
        *env.local_mutation_fixture.lock().unwrap() = Some(fixture.clone());
        let (sid, first) = local_run(&host);
        host.release_run(&first);
        host.set_permission(super::local::PermissionRequest {
            session_id: sid.clone(),
            mode: super::super::tools::policy::PermissionMode::FullAuto,
            confirmed_full_access: true,
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        })
        .await
        .unwrap();
        let mut run = control(&sid);
        let state = Arc::get_mut(&mut run).unwrap();
        state.local_enabled = true;
        state.local_home = Some(admitted_home.clone());
        if !cancel {
            state.deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        }
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let call = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
            "toolCallId":"interrupted-mutation","name":"rebind_tool","args":{"tool":"codex"}});
        let mut work = tokio::spawn({
            let host = host.clone();
            let run = run.clone();
            async move { host.dispatch_tool(&run, &call).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                biased;
                _ = fixture.written.notified() => {},
                result = &mut work => match result {
                    Ok(Err(error)) => panic!("mutation refused before the journaled write: {error}"),
                    Ok(Ok(_)) => panic!("mutation returned before the journaled write"),
                    Err(error) => panic!("mutation worker failed before the journaled write: {error}"),
                },
            }
        })
        .await
        .expect("mutation did not reach its journaled write before interruption");
        if cancel {
            run.cancel.send_replace(true);
        }
        let error = work.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            if cancel {
                CANCELLED
            } else {
                "assistant_run_timeout"
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"changed");
        let pending = host.local_task_state(&identity().owner, &sid);
        assert_eq!(
            pending["changed"], true,
            "committed mutation lost its verification debt"
        );
        assert_eq!(
            pending["targets"],
            json!([{
                "tool":"codex","folder":admitted_home,"verified":false
            }])
        );
        // The runtime saw cancellation, not the successful changed result.
        // Its empty snapshot must not erase independently observed Host debt.
        host.keep_local_task(&run, &json!({"localTaskState":null}));
        assert_eq!(host.local_task_state(&identity().owner, &sid), pending);
        host.release_run(&run);
        let changes = host
            .local_changes(
                DeleteRequest {
                    session_id: sid,
                    expected_owner: identity().owner,
                    expected_generation: host.generation(),
                },
                false,
            )
            .await
            .unwrap();
        assert!(changes.iter().any(|change| change.changed));
        #[cfg(unix)]
        verify_interrupted_debt_in_canonical_home(&env, &host, &run.session_id).await;
    }
}

#[cfg(unix)]
async fn verify_interrupted_debt_in_canonical_home(
    env: &Arc<FixedEnvironment>,
    host: &Arc<AssistantHost>,
    sid: &str,
) {
    use std::os::unix::fs::PermissionsExt;
    struct RestoreShell(Option<std::ffi::OsString>);
    impl Drop for RestoreShell {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("SHELL", value),
                None => std::env::remove_var("SHELL"),
            }
        }
    }
    let _restore = RestoreShell(std::env::var_os("SHELL"));
    let cli = env.config.join("fixture-codex");
    let shell = env.config.join("fixture-shell");
    std::fs::write(&cli, "#!/bin/sh\nprintf 'codex-cli 0.0.1\\n'\n").unwrap();
    let quoted = cli.to_string_lossy().replace('\'', "'\\''");
    std::fs::write(&shell, format!("#!/bin/sh\nprintf '\\n__OFOX_BIN__%s\\n' '{quoted}'\nprintf '__OFOX_PATH__%s\\n' '/usr/bin:/bin'\n")).unwrap();
    for path in [&cli, &shell] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("SHELL", &shell);
    let alias = env.config.join("home-alias");
    std::os::unix::fs::symlink(&env.config, &alias).unwrap();
    *env.local_home.lock().unwrap() = Some(alias.clone());
    let mut next = control(sid);
    let state = Arc::get_mut(&mut next).unwrap();
    state.local_enabled = true;
    let admitted_home = resolve_local_test_path(&env.config).unwrap();
    state.local_home = Some(admitted_home.clone());
    host.register_run(next.clone(), host.generation(), || Ok(()))
        .unwrap();
    let call = |id: &str, name: &str, args: Value| json!({"sessionId":sid,"runId":next.run_id,"operationId":next.operation_id,"toolCallId":id,"name":name,"args":args});
    let status = host
        .dispatch_tool(
            &next,
            &call("canonical-status", "tool_status", json!({"tool":"chatgpt"})),
        )
        .await
        .unwrap();
    assert_eq!(status["folder"], json!(admitted_home));
    let verified = host
        .dispatch_tool(
            &next,
            &call(
                "verify-original",
                "verify_in_folder",
                json!({"tool":"chatgpt","folder":alias}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(verified["verified"], true, "{verified}");
    host.keep_local_task(&next, &json!({"localTaskState":null}));
    assert!(host.local_task_state(&identity().owner, sid).is_null());
    host.release_run(&next);
}

#[tokio::test]
#[serial_test::serial]
async fn a_known_no_effect_mutation_failure_does_not_create_host_debt() {
    let _sandbox = crate::test_sandbox::sandbox();
    let (_tmp, env, host) = fixture_with_env();
    *env.local_mutation_fixture.lock().unwrap() = Some(Arc::new(LocalMutationFixture {
        path: env.config.join("config/unchanged"),
        change: false,
        fail_after_write: None,
        written: Default::default(),
    }));
    let (sid, first) = local_run(&host);
    host.release_run(&first);
    host.set_permission(super::local::PermissionRequest {
        session_id: sid.clone(),
        mode: super::super::tools::policy::PermissionMode::FullAuto,
        confirmed_full_access: true,
        expected_owner: identity().owner,
        expected_generation: host.generation(),
    })
    .await
    .unwrap();
    let mut run = control(&sid);
    Arc::get_mut(&mut run).unwrap().local_enabled = true;
    host.register_run(run.clone(), host.generation(), || Ok(()))
        .unwrap();
    let call = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"no-effect","name":"rebind_tool","args":{"tool":"codex"}});
    assert_eq!(
        host.dispatch_tool(&run, &call).await.unwrap_err(),
        "assistant_local_unavailable"
    );
    assert!(host.local_task_state(&identity().owner, &sid).is_null());
    host.keep_local_task(&run, &json!({"localTaskState":null}));
    assert!(host.local_task_state(&identity().owner, &sid).is_null());
    host.release_run(&run);
}

#[tokio::test]
#[serial_test::serial]
async fn lifecycle_unknown_outcome_keeps_host_debt_after_a_real_effect() {
    let _sandbox = crate::test_sandbox::sandbox();
    let (_tmp, env, host) = fixture_with_env();
    let path = env.config.join("config/installation-fixture");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    *env.local_mutation_fixture.lock().unwrap() = Some(Arc::new(LocalMutationFixture {
        path: path.clone(),
        change: true,
        fail_after_write: Some("unknown_outcome"),
        written: Default::default(),
    }));
    let (sid, first) = local_run(&host);
    host.release_run(&first);
    host.set_permission(super::local::PermissionRequest {
        session_id: sid.clone(),
        mode: super::super::tools::policy::PermissionMode::FullAuto,
        confirmed_full_access: true,
        expected_owner: identity().owner,
        expected_generation: host.generation(),
    })
    .await
    .unwrap();
    let mut run = control(&sid);
    Arc::get_mut(&mut run).unwrap().local_enabled = true;
    host.register_run(run.clone(), host.generation(), || Ok(()))
        .unwrap();
    let call = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
        "toolCallId":"unknown-install","name":"install_tool","args":{"tool":"codex"}});
    assert_eq!(
        host.dispatch_tool(&run, &call).await.unwrap_err(),
        "assistant_mutation_outcome_unknown"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"changed");
    host.keep_local_task(&run, &json!({"localTaskState":null}));
    assert_eq!(
        host.local_task_state(&identity().owner, &sid)["changed"],
        true
    );
    host.release_run(&run);
}
