use crate::error::AppError;

pub struct InstructionPatch {
    pub text: String,
    pub owned_segment: Option<String>,
}

const BEGIN: &str = "<!-- OFOX:BEGIN instructions -->";
const END: &str = "<!-- OFOX:END instructions -->";

pub fn patch_instructions(
    current: &str,
    owned: Option<&str>,
    content: Option<&str>,
) -> Result<InstructionPatch, AppError> {
    let conflict = || AppError::Config("library.instructionsConflict".into());
    if content.is_some_and(|text| text.contains("<!-- OFOX:")) {
        return Err(conflict());
    }
    let begins = current.matches(BEGIN).count();
    let ends = current.matches(END).count();
    if begins != ends || begins > 1 {
        return Err(conflict());
    }
    let (start, end) = match owned {
        Some(segment) => {
            let block = segment
                .strip_prefix("\r\n")
                .or_else(|| segment.strip_prefix('\n'))
                .unwrap_or(segment);
            if (!block.starts_with(&format!("{BEGIN}\n"))
                && !block.starts_with(&format!("{BEGIN}\r\n")))
                || (!block.ends_with(&format!("{END}\n"))
                    && !block.ends_with(&format!("{END}\r\n")))
                || segment.matches(BEGIN).count() != 1
                || segment.matches(END).count() != 1
            {
                return Err(conflict());
            }
            if begins != 1 || segment.is_empty() || current.matches(segment).count() != 1 {
                return Err(conflict());
            }
            let start = current.find(segment).ok_or_else(conflict)?;
            (start, start + segment.len())
        }
        None => {
            if begins != 0 {
                return Err(conflict());
            }
            (current.len(), current.len())
        }
    };
    let prefix = &current[..start];
    let suffix = &current[end..];
    let segment = content.map(|text| {
        let eol = if current.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let separator = if prefix.is_empty() || prefix == "\u{feff}" || prefix.ends_with('\n') {
            ""
        } else {
            eol
        };
        let normalized = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', eol);
        format!("{separator}{BEGIN}{eol}{normalized}{eol}{END}{eol}")
    });
    Ok(InstructionPatch {
        text: format!("{prefix}{}{suffix}", segment.as_deref().unwrap_or("")),
        owned_segment: segment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_withdraw_preserve_every_user_byte() {
        for original in [
            "",
            "\u{feff}personal\r\n",
            "personal without newline",
            "before\n\nafter\n",
        ] {
            let patch = patch_instructions(original, None, Some("shared\nsecond")).unwrap();
            assert!(patch.text.starts_with(original));
            let removed =
                patch_instructions(&patch.text, patch.owned_segment.as_deref(), None).unwrap();
            assert_eq!(removed.text, original);
        }
    }

    #[test]
    fn preserve_edits_outside_owned_block_but_reject_owned_edits() {
        let patch = patch_instructions("user\n", None, Some("shared")).unwrap();
        let current = format!("{}new user content\n", patch.text);
        let changed =
            patch_instructions(&current, patch.owned_segment.as_deref(), Some("updated")).unwrap();
        assert!(changed.text.starts_with("user\n"));
        assert!(changed.text.ends_with("new user content\n"));
        assert!(patch_instructions(
            &current.replace("shared", "manual edit"),
            patch.owned_segment.as_deref(),
            None
        )
        .is_err());
    }

    #[test]
    fn reject_foreign_broken_duplicate_and_embedded_markers() {
        let valid = patch_instructions("", None, Some("shared")).unwrap();
        assert!(patch_instructions(&valid.text, None, Some("updated")).is_err());
        assert!(patch_instructions("<!-- OFOX:BEGIN instructions -->", None, Some("x")).is_err());
        assert!(
            patch_instructions(&valid.text.repeat(2), valid.owned_segment.as_deref(), None)
                .is_err()
        );
        assert!(patch_instructions("", None, Some("<!-- OFOX:END instructions -->")).is_err());
    }

    #[test]
    fn a_corrupt_receipt_cannot_claim_arbitrary_text() {
        let valid = patch_instructions("", None, Some("shared")).unwrap();
        assert!(patch_instructions(&valid.text, Some("shared"), None).is_err());
    }
}
