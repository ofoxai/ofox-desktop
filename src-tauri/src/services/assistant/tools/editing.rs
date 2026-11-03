//! Preserve whole secret-bearing lines when applying edits to scrubbed text.
//! A change to a redacted line is ambiguous and must use a structured tool.
use crate::redact::{scrub, REDACTED};
pub fn preserve_credentials(original: &str, edited: &str) -> Result<String, String> {
    if scrub(edited) != edited {
        return Err("assistant_secret_write_denied".into());
    }
    let mut protected: Vec<_> = original
        .split_inclusive('\n')
        .filter_map(|line| {
            let clean = scrub(line);
            (clean != line).then(|| (clean.into_owned(), line, false))
        })
        .collect();
    let mut result = String::new();
    for line in edited.split_inclusive('\n') {
        if line.contains(REDACTED) {
            let Some((_, original, used)) = protected
                .iter_mut()
                .find(|(clean, _, used)| !(*used) && clean == line)
            else {
                return Err("assistant_secret_write_denied".into());
            };
            *used = true;
            result.push_str(original);
        } else {
            result.push_str(line);
        }
    }
    // Even deleting a credential is a structured operation, never an accidental
    // consequence of the model rewriting text it could not see.
    if protected.iter().any(|(_, _, used)| !*used) {
        return Err("assistant_secret_write_denied".into());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_preserve_hidden_credentials_and_reject_ambiguous_changes() {
        let original = "api_key='super-secret'\nmodel='old'\n";
        let edited = "api_key='«redacted»'\nmodel='new'\n";
        assert_eq!(
            preserve_credentials(original, edited).unwrap(),
            "api_key='super-secret'\nmodel='new'\n"
        );
        for edit in [
            "model='new'\n",
            "api_key='another-secret'\nmodel='new'\n",
            "api_key='«redacted»'; extra\nmodel='new'\n",
        ] {
            assert!(preserve_credentials(original, edit).is_err());
        }
        assert!(preserve_credentials("", edited).is_err());
    }
}
