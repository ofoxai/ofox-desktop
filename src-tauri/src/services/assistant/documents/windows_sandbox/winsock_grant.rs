//! CI-only, test-only: for one comparison launch, lets the private runtime
//! capability read the Winsock catalog, then restores the exact original DACL.
//! It changes a machine-wide HKLM ACL, so `apply` itself refuses to run unless
//! GITHUB_ACTIONS is "true" (a disposable hosted runner).
use super::*;
use std::collections::BTreeMap;
use windows::Win32::Security::{
    Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    },
    ACL, CONTAINER_INHERIT_ACE,
};

/// The catalog root WSAStartup reads; the inheritable ACE reaches its subtree.
const CATALOG: &str = r"SYSTEM\CurrentControlSet\Services\WinSock2\Parameters";
/// KEY_READ.
const KEY_READ_ACCESS: u32 = 0x0002_0019;
/// Far above a real catalog (about a hundred keys); exceeding it is an error.
const SUBTREE_LIMIT: usize = 10_000;

fn object_name(relative: &str) -> String {
    if relative.is_empty() {
        format!(r"MACHINE\{CATALOG}")
    } else {
        format!(r"MACHINE\{CATALOG}\{relative}")
    }
}

/// The DACL of a registry object as SDDL, for exact before/after comparison.
fn dacl_sddl(name: &str) -> Result<String, String> {
    let name = wide(name)?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 || descriptor.0.is_null() {
        return Err(UNAVAILABLE.into());
    }
    let descriptor = Local(descriptor.0);
    let mut text = PWSTR::null();
    unsafe {
        win(ConvertSecurityDescriptorToStringSecurityDescriptorW(
            PSECURITY_DESCRIPTOR(descriptor.0),
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            None,
        ))?;
    }
    let allocation = Local(text.0.cast());
    let result = unsafe { text.to_string() }.map_err(|_| UNAVAILABLE.to_owned());
    drop(allocation);
    result
}

/// Every key under the catalog root (including it), by relative path, with
/// its DACL as SDDL. The restore proof compares the whole map.
fn subtree_sddl() -> Result<BTreeMap<String, String>, String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let mut snapshot = BTreeMap::new();
    let mut pending = vec![String::new()];
    while let Some(relative) = pending.pop() {
        if snapshot.len() >= SUBTREE_LIMIT {
            return Err(UNAVAILABLE.into());
        }
        snapshot.insert(relative.clone(), dacl_sddl(&object_name(&relative))?);
        let path = object_name(&relative)["MACHINE\\".len()..].to_owned();
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(&path, KEY_READ_ACCESS)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        for name in key.enum_keys() {
            let name = name.map_err(|_| UNAVAILABLE.to_owned())?;
            pending.push(if relative.is_empty() {
                name
            } else {
                format!(r"{relative}\{name}")
            });
        }
    }
    Ok(snapshot)
}

/// `apply` failed. `restored`: the original subtree is proven intact (true
/// also when nothing was ever changed).
pub(super) struct GrantFailure {
    pub(super) restored: bool,
}

pub(super) struct WinsockCatalogGrant {
    /// Owns the original security descriptor that `dacl` points into.
    _original: Local,
    dacl: *mut ACL,
    before: BTreeMap<String, String>,
    restored: bool,
}

impl WinsockCatalogGrant {
    pub(super) fn apply(capability: PSID) -> Result<Self, GrantFailure> {
        let unchanged = || GrantFailure { restored: true };
        if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
            return Err(unchanged());
        }
        let before = subtree_sddl().map_err(|_| unchanged())?;
        let name = wide(object_name("")).map_err(|_| unchanged())?;
        let (mut dacl, mut descriptor) = (std::ptr::null_mut(), PSECURITY_DESCRIPTOR::default());
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_REGISTRY_KEY,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut descriptor,
            )
        };
        if status.0 != 0 || descriptor.0.is_null() {
            return Err(unchanged());
        }
        let original = Local(descriptor.0);
        if dacl.is_null() {
            return Err(unchanged());
        }
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: KEY_READ_ACCESS,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: CONTAINER_INHERIT_ACE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: PWSTR(capability.0.cast()),
            },
        };
        let mut updated = std::ptr::null_mut();
        let status = unsafe { SetEntriesInAclW(Some(&[access]), Some(dacl), &mut updated) };
        if status.0 != 0 || updated.is_null() {
            return Err(unchanged());
        }
        let updated = Local(updated.cast());
        let mut grant = Self {
            _original: original,
            dacl,
            before,
            restored: false,
        };
        // SetNamedSecurityInfo propagates the inheritable ACE to child keys.
        let status = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_REGISTRY_KEY,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(updated.0.cast::<ACL>()),
                None,
            )
        };
        if status.0 != 0 {
            // A partial change is possible; report whether it was undone.
            return Err(GrantFailure {
                restored: grant.restore(),
            });
        }
        Ok(grant)
    }

    /// Writes back the original DACL and proves every key of the catalog
    /// subtree has its pre-grant DACL again (SDDL equality). Only a proven
    /// restore is cached, so `Drop` retries after a failure.
    pub(super) fn restore(&mut self) -> bool {
        if self.restored {
            return true;
        }
        let proven = (|| {
            let name = wide(object_name(""))?;
            let status = unsafe {
                SetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_REGISTRY_KEY,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(self.dacl.cast_const()),
                    None,
                )
            };
            if status.0 != 0 {
                return Err(UNAVAILABLE.to_owned());
            }
            Ok::<_, String>(subtree_sddl()? == self.before)
        })()
        .unwrap_or(false);
        self.restored = proven;
        proven
    }
}

impl Drop for WinsockCatalogGrant {
    fn drop(&mut self) {
        if !self.restored {
            self.restore();
        }
    }
}
