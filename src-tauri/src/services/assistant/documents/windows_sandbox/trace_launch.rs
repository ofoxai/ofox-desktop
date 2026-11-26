//! Test-only: the launch the CDB observer attaches to. It reproduces the
//! configuration that still fails in WSAStartup with the Winsock catalog
//! readable (native run 44142596063, row B): console-free (DETACHED_PROCESS),
//! the private runtime capability only, and the CI-only temporary ACE on the
//! Winsock catalog subtree, restored and proven when the launch ends.
use super::*;

fn record(observation: &Observer, update: impl FnOnce(&mut probe::LoaderDiagnostic)) {
    observation.evidence(|evidence| update(evidence.loader_diagnostic.get_or_insert_default()));
}

/// Restores the catalog on every path when the launch ends and records whether
/// every key of the subtree was proven back to its original DACL.
pub(super) struct HeldGrant {
    grant: winsock_grant::WinsockCatalogGrant,
    observation: Observer,
}
impl Drop for HeldGrant {
    fn drop(&mut self) {
        let restored = self.grant.restore();
        record(&self.observation, |diagnostic| {
            diagnostic.winsock_grant_restored = Some(restored)
        });
    }
}

pub(super) fn create(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    profile: &Profile,
    cap: &RuntimeCapability,
    observation: &Observer,
) -> Result<(SuspendedLaunch, HeldGrant), String> {
    let console = ConsoleMode::Detached;
    record(observation, |diagnostic| {
        diagnostic.launch_profile = Some("detached_runtime_only_winsock_grant");
        diagnostic.creation_flags = Some(console.creation_flags().0);
        diagnostic.winsock_grant_applied = Some(false);
    });
    // apply() refuses anywhere but a disposable GitHub Actions runner.
    let grant = winsock_grant::WinsockCatalogGrant::apply(cap.sid).map_err(|failure| {
        record(observation, |diagnostic| {
            diagnostic.winsock_grant_restored = Some(failure.restored)
        });
        UNAVAILABLE.to_owned()
    })?;
    let grant = HeldGrant {
        grant,
        observation: observation.clone(),
    };
    record(observation, |diagnostic| {
        diagnostic.winsock_grant_applied = Some(true)
    });
    let caps = ProcessCapabilities::runtime_only(cap);
    let suspended = create_suspended_capabilities(
        exe,
        args,
        cwd,
        home,
        profile,
        &caps,
        ContainerPolicy::Lpac,
        console,
        observation,
        None,
    )?;
    // The registry gate must see the granted catalog read and denied user keys.
    verify_token_entries(
        raw(&suspended.process.process),
        profile.sid.0,
        cap.sid,
        &caps.entries(),
        RegistryScope::RuntimeOnly {
            winsock_readable: true,
        },
        observation,
    )?;
    Ok((suspended, grant))
}
