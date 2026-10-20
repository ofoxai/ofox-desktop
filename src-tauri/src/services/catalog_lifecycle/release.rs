use super::Failure;
use serde_json::Value;
#[derive(Clone, Debug)]
pub(super) struct Release {
    pub version: String,
    pub url: String,
    pub integrity: String,
}
pub(super) fn parse(manifest: &Value, node: &str) -> Result<Release, Failure> {
    use base64::Engine;
    let version = manifest["version"]
        .as_str()
        .ok_or(Failure::InvalidPackage)?;
    let integrity = manifest["dist"]["integrity"]
        .as_str()
        .ok_or(Failure::InvalidPackage)?;
    let url = format!("https://registry.npmjs.org/@github/copilot/-/copilot-{version}.tgz");
    if manifest["name"] != "@github/copilot"
        || manifest["bin"]["copilot"] != "npm-loader.js"
        || manifest["dist"]["tarball"] != url
        || semver::Version::parse(version)
            .ok()
            .is_none_or(|version| !version.pre.is_empty())
        || manifest
            .get("scripts")
            .is_some_and(|value| value.as_object().is_none_or(|object| !object.is_empty()))
        || manifest
            .get("engines")
            .and_then(|engines| engines.get("node"))
            .is_some_and(|engine| {
                engine.as_str().is_none_or(|engine| {
                    crate::commands::node_requirement::engine_satisfied(node, engine) != Some(true)
                })
            })
        || integrity
            .strip_prefix("sha512-")
            .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
            .is_none_or(|bytes| bytes.len() != 64)
    {
        return Err(Failure::InvalidPackage);
    }
    Ok(Release {
        version: version.into(),
        url,
        integrity: integrity.into(),
    })
}
pub(super) fn verify(release: &Release, bytes: &[u8]) -> Result<(), Failure> {
    use base64::Engine;
    use sha2::{Digest, Sha512};
    if release.integrity
        == format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes))
        )
    {
        Ok(())
    } else {
        Err(Failure::Changed)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::{Digest, Sha512};
    #[test]
    fn pins_official_package_bytes_and_rejects_redirected_or_scripted_recipes() {
        let hash =
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(b"frozen tarball"));
        let manifest = serde_json::json!({"name":"@github/copilot","version":"1.0.98","bin":{"copilot":"npm-loader.js"},"dist":{"tarball":"https://registry.npmjs.org/@github/copilot/-/copilot-1.0.98.tgz","integrity":format!("sha512-{hash}")}});
        let release = parse(&manifest, "22.12.0").unwrap();
        assert_eq!(release.version, "1.0.98");
        assert!(verify(&release, b"frozen tarball").is_ok());
        assert!(verify(&release, b"replacement bytes").is_err());
        for bad in [
            {
                let mut v = manifest.clone();
                v["name"] = "other".into();
                v
            },
            {
                let mut v = manifest.clone();
                v["scripts"] = serde_json::json!({"install":"unknown"});
                v
            },
            {
                let mut v = manifest.clone();
                v["dist"]["tarball"] = "https://elsewhere.invalid/copilot.tgz".into();
                v
            },
            {
                let mut v = manifest.clone();
                v["dist"]["integrity"] = "sha1-unknown".into();
                v
            },
            {
                let mut v = manifest.clone();
                v["engines"] = serde_json::json!({"node":">=24"});
                v
            },
        ] {
            assert!(parse(&bad, "22.12.0").is_err());
        }
    }
}
