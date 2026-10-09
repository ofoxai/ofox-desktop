# Bound tool lifecycle

Installation evidence, the retained OFox binding, and the live connection
configuration are separate states. Deleting an application never unbinds it.

## Home page

Confirmed missing installations move to a collapsed section below the installed
tools. Its count remains part of the total bound count. Detection failures and
found-but-broken executables stay in the main list. Every row retains Manage as
its last action. The preceding action is Open, Reinstall/Install instructions,
Repair installation, Check again, or Restore binding according to the state.
Actual host installer capabilities determine whether installation is automatic.

Local state is checked on mount, focus, refresh, install completion, configuration
changes, and OFox region changes. Only `installationStatus: notInstalled` can
move a row to the missing section; a missing version is insufficient evidence.
Installation failures precede configuration problems in the row action. More
detailed configuration diagnostics remain available through Manage.

## Configuration changes

Only configured bindings permit ordinary model saves and connection tests.
Missing fields/files can be restored explicitly from Manage, using the saved
model selection and selected OFox apex. Modified fields, invalid configuration,
or failed reads cannot be overwritten through recovery. The saved binding
snapshot and other holders of shared Codex/ChatGPT configuration are retained.

Unbinding previews its changes before confirmation. Files which no longer exist
are skipped (`configAlreadyMissing`), rather than rebuilt from the snapshot.
Shared configuration stays bound until its final holder is removed. Errors keep
the home-page binding available for retry. Unbinding does not uninstall software,
delete conversations/caches, or revoke remote API keys.

Adding tools binds only newly selected tools and merges their successful results
with existing bindings. Startup repair, model saves, and opening a configuration
folder do not recreate deliberately deleted configuration. Recovery is a separate
explicit operation; installation completion itself does not recover bindings.

Startup, first-launch region detection, and region changes reconcile only
existing OFox endpoint fields. Saved models, API keys, shared holders, and
binding snapshots are preserved. Missing files/fields stay missing, and custom
endpoints produce a conflict instead of being overwritten. Each tool is attempted
independently so one conflict cannot block another tool's safe migration.

The bound-tools settings mirror uses an atomic field update. Ordinary preference
saves retain the backend's current region, key metadata, and binding list, even
when the renderer submits an older settings snapshot.

## Local interfaces

- `get_tool_versions`: adds `installationStatus` (`installed`, `notInstalled`,
  `unknown`), independently of version/update status. Found installations can
  have an unavailable version or an error. Failed probes are not uninstall evidence.
- `get_tool_install_capabilities`: returns tool IDs supported by this host's
  automatic installer. Other tools link to upstream download/instructions.
- `get_tool_binding_status { app }`: returns `status` (`configured`, `missing`,
  `modified`, `unknown`), safe `message`, and `missingFiles`. No config values or
  credentials cross this interface.
- `ofox_restore_tool_binding { app, stillBound }`: explicitly restores missing
  OFox configuration, revalidating it under the same locks used for binding writes.
- Existing unbind reports add the `configAlreadyMissing` warning. The binding
  list and snapshot storage formats are unchanged.
- `save_bound_tools { boundTools }`: updates only the settings mirror of the bound
  tool list, without round-tripping unrelated preferences or backend state.

Windows host entry points and the manual acceptance matrix are documented in
[Windows testing](windows-testing.md).
