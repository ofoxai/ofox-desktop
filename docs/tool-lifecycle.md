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

### Unbind restores only what OFox still owns

Every OFox write to a bound tool (bind, key replacement, model save, missing
binding restore, region reconciliation, the legacy Codex migration) records the
managed fields it wrote in the binding record (`written`, per managed file). Keys
and tokens are stored as SHA-256 digests only. A field OFox left untouched keeps
its earlier entry, so a value the user changed is never recorded as OFox's when
OFox later rewrites the file around it. If the record cannot be saved, the write
is rolled back.

On unbind each managed field is judged on its own:

- **Route** (provider id, the OFox provider entry, base URL, token/key): restored
  when it is still what OFox wrote or still looks like OFox (the `ofox` provider
  id, an OFox gateway URL, an `sk-of-` key). A route the user pointed at their own
  provider is kept. OpenClaw's default model names its provider
  (`ofox-openclaw/<model>`), so it is a route of its own.
- **Model** (Codex `model`, `ANTHROPIC_MODEL`, `GEMINI_MODEL`, Hermes
  `model.default`): restored when it is still what OFox wrote, or when its route
  is restored. A model picked in the agent goes back with the OFox route, and an
  OFox model id never stays behind under the user's own provider.
- **Preference** (Codex `model_reasoning_effort`, `disable_response_storage`,
  Gemini's sign-in type, OpenClaw `models.mode`, OpenCode `$schema`): restored only
  when it is still what OFox wrote.

Kept fields that differ from the pre-bind value are listed in the report's
`keptKeys` (names only, no values) with the `userValueKept` warning; preview and
unbind report the same. Records written before this rule (no `written`), and
files whose configuration directory moved since binding, restore every managed
field as before.

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

## Sign-out and account switching

API keys are billed to the organization that created them, so a signed-out
account must stop paying and a new account must not silently spend another's
credits. Signing out does not restore pre-bind settings; Unbind does that.

- **Sign out** revokes this account's keys on the server and marks them revoked
  locally. Tool configuration files are left untouched: they still point at OFox
  with a key that no longer works, so bound tools stop until the next sign-in.
  The keychain secret is kept only so the configuration can be verified later.
- **Offline sign-out** cannot revoke on the server. Those keys keep working, and
  the sign-out result lists them. They are queued and deleted the next time the
  owning account signs in on this machine.
- **Sign-in** is followed by a check of every bound tool when the home page loads
  (the renderer calls `ofox_reconcile_tool_keys` after refreshing the profile; there is
  no separate backend startup hook). A key that is revoked or owned
  by another account is replaced with a new key issued for the current account,
  including when the same account signs back in. Only the key is replaced; model
  and protocol settings stay. The configuration is verified with the old key
  first, and a tool whose configuration was edited by the user is reported, not
  overwritten.
- **Keys of tools that are no longer bound** are dropped locally and queued for
  deletion rather than replaced.
- **Legacy keys** without a recorded owner are adopted by whichever account signs
  in first.
- **Region changes** do not revoke keys. They only reconcile OFox endpoint fields
  (see Configuration changes above).
- **Unbind** restores the pre-bind connection settings but does not revoke the
  key.

## Local interfaces

- `get_tool_versions`: adds `installationStatus` (`installed`, `notInstalled`,
  `unknown`), independently of version/update status. Found installations can
  have an unavailable version or an error. Failed probes are not uninstall evidence.
  Launch-shell startup stderr does not fail a completed lookup; when the lookup
  itself fails, an executable found by the path scan still counts as installed.
- `get_tool_install_capabilities`: returns tool IDs supported by this host's
  automatic installer. Other tools link to upstream download/instructions.
- `get_tool_binding_status { app }`: returns `status` (`configured`, `missing`,
  `modified`, `unknown`), safe `message`, and `missingFiles`. No config values or
  credentials cross this interface.
- `ofox_restore_tool_binding { app, stillBound }`: explicitly restores missing
  OFox configuration, revalidating it under the same locks used for binding writes.
- Existing unbind reports add the `configAlreadyMissing` and `userValueKept`
  warnings and `keptKeys`. The binding record gains an optional `written` map
  (record version unchanged; older builds ignore it); the binding list format is
  unchanged.
- `ofox_logout`: revokes this account's keys, then clears the sign-in. Returns
  `stopped` tools and `notRevoked` tools (server revoke failed, queued for deletion).
- `ofox_reconcile_tool_keys`: called after sign-in; replaces keys that are revoked
  or owned by another account. Returns `switched` and `failed` tools. Makes no
  network request when nothing needs replacing.
- `save_bound_tools { boundTools }`: updates only the settings mirror of the bound
  tool list, without round-tripping unrelated preferences or backend state.

Windows host entry points and the manual acceptance matrix are documented in
[Windows testing](windows-testing.md).
