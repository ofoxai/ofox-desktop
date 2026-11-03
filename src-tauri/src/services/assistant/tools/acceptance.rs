//! Offline macOS acceptance: real fnm and two already-installed Node binaries.
//! No downloads, provider requests, or real user configuration writes.
use super::{approval, diagnostics, journal::Journal, policy, scope::Scope, transaction};
use crate::{
    app_config::BindableTool,
    commands::{node_launcher, tool_update},
};
use std::os::unix::fs::PermissionsExt;
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
fn node_version(node: &Path) -> String {
    let out = std::process::Command::new(node)
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().into()
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "offline manual acceptance: set OFOX_TEST_FNM, OFOX_TEST_NODE_OLD, OFOX_TEST_NODE_NEW, OFOX_TEST_OPENCODE"]
async fn real_fnm_problem_folder_repair_verify_and_restart_restore_in_each_mode() {
    let fnm = std::path::PathBuf::from(std::env::var_os("OFOX_TEST_FNM").expect("fnm path"));
    let old_node =
        std::path::PathBuf::from(std::env::var_os("OFOX_TEST_NODE_OLD").expect("older Node path"));
    let new_node =
        std::path::PathBuf::from(std::env::var_os("OFOX_TEST_NODE_NEW").expect("newer Node path"));
    let opencode = std::path::PathBuf::from(
        std::env::var_os("OFOX_TEST_OPENCODE").expect("installed OpenCode binary"),
    );
    let old_version = node_version(&old_node);
    let new_version = node_version(&new_node);
    assert_ne!(old_version, new_version);
    struct Shell(Option<std::ffi::OsString>);
    impl Drop for Shell {
        fn drop(&mut self) {
            if let Some(value) = &self.0 {
                std::env::set_var("SHELL", value);
            } else {
                std::env::remove_var("SHELL");
            }
        }
    }
    let _shell = Shell(std::env::var_os("SHELL"));
    std::env::set_var("SHELL", "/bin/zsh");
    for mode in [
        policy::PermissionMode::Default,
        policy::PermissionMode::AutoEdit,
        policy::PermissionMode::FullAuto,
    ] {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home();
        let manager = home.join(".local/share/fnm");
        for (version, node) in [(&old_version, &old_node), (&new_version, &new_node)] {
            let bin = manager
                .join("node-versions")
                .join(version)
                .join("installation/bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::copy(node, bin.join("node")).unwrap();
        }
        let package = manager
            .join("node-versions")
            .join(&new_version)
            .join("installation/lib/node_modules/opencode-ai/bin/opencode.exe");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::copy(&opencode, &package).unwrap();
        std::os::unix::fs::symlink(
            &package,
            manager
                .join("node-versions")
                .join(&new_version)
                .join("installation/bin/opencode"),
        )
        .unwrap();
        let status = std::process::Command::new(&fnm)
            .args(["default", &new_version])
            .env_clear()
            .env("HOME", home)
            .env("FNM_DIR", &manager)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let original=format!("export PATH=/usr/bin:/bin:/usr/sbin:/sbin\nexport FNM_DIR={}\nunset FNM_MULTISHELL_PATH\neval \"$({} env --use-on-cd --shell zsh)\"\n",quote(&manager),quote(&fnm));
        write(&home.join(".zshrc"), &original);
        let problem = home.join("problem folder");
        std::fs::create_dir(&problem).unwrap();
        std::fs::write(problem.join(".nvmrc"), &old_version).unwrap();
        let before = diagnostics::probe(BindableTool::OpenCode, &problem, home).await;
        assert_eq!(before["verified"], false, "{before}");
        let decision = policy::decide(
            true,
            mode,
            policy::Operation {
                category: policy::Category::WriteConfig,
                read_only_command: false,
                forbidden: false,
            },
        );
        if decision == policy::Decision::Confirm {
            let broker = approval::Broker::default();
            let binding = approval::Binding {
                owner: "test".into(),
                session_id: "session".into(),
                run_id: "run".into(),
                operation_id: "op".into(),
                generation: 1,
            };
            let (id, rx) = broker.request(binding.clone()).unwrap();
            broker
                .respond(&id, &binding, approval::Choice::Once)
                .unwrap();
            let (cancel, _) = tokio::sync::watch::channel(false);
            assert_eq!(
                broker
                    .wait(&id, rx, cancel.subscribe(), Duration::from_secs(1))
                    .await
                    .unwrap(),
                approval::Choice::Once
            );
        }
        let install = tool_update::probe_executable_in("opencode", home)
            .await
            .unwrap();
        assert!(!install.version.is_empty());
        let (_, cancel) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(transaction::Context {
            journal: Mutex::new(
                Journal::open(&home.join("changes"), &[9; 32], "fnm/session").unwrap(),
            ),
            tool: "opencode".into(),
            apex: None,
            operation: "repair_launcher".into(),
            scope: Scope::new(home.into(), vec![home.into()], vec![]).unwrap(),
            cancel,
            deadline: tokio::time::Instant::now() + Duration::from_secs(60),
            error: Mutex::new(None),
            observed: AtomicBool::new(false),
        });
        transaction::ACTIVE
            .scope(ctx.clone(), async {
                node_launcher::ensure("opencode", &install, home, "/bin/zsh")
            })
            .await
            .unwrap();
        assert!(ctx.error.lock().unwrap().is_none());
        drop(ctx);
        let after = diagnostics::probe(BindableTool::OpenCode, &problem, home).await;
        assert_eq!(after["verified"], true, "{after}");
        assert_eq!(after["version"].as_str().unwrap(), install.version);
        let node = std::process::Command::new("/bin/zsh")
            .args(["-l", "-i", "-c", "node --version"])
            .current_dir(&problem)
            .output()
            .unwrap();
        assert!(node.status.success());
        assert!(String::from_utf8_lossy(&node.stdout).contains(&old_version));
        Journal::open(&home.join("changes"), &[9; 32], "fnm/session")
            .unwrap()
            .restore_files()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(".zshrc")).unwrap(),
            original
        );
        assert!(!node_launcher::launcher_dir(home).join("opencode").exists());
        assert_eq!(
            diagnostics::probe(BindableTool::OpenCode, &problem, home).await["verified"],
            false
        );
        eprintln!("fnm acceptance passed: {mode:?}, {old_version} -> {new_version}, verify and restart restore");
    }
}
