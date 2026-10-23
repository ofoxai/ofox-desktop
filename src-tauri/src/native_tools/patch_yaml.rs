//! Cordis expressions must not be flattened into literal values. Inspect the
//! actual YAML token stream: serde_yaml::Value loses tags expanded to URI names,
//! while a substring check mistakes comments and quoted/block scalars for tags.
use crate::error::AppError;
use std::mem::MaybeUninit;
use unsafe_libyaml as unsafe_yaml;

struct Scanner(Box<MaybeUninit<unsafe_yaml::yaml_parser_t>>);

impl Drop for Scanner {
    fn drop(&mut self) {
        // SAFETY: Scanner is constructed only after successful initialization.
        // Its heap allocation stays at the same address until this deletion.
        unsafe { unsafe_yaml::yaml_parser_delete(self.0.as_mut_ptr()) };
    }
}

pub(super) fn validate_no_tags(text: &str) -> Result<(), AppError> {
    let invalid = || AppError::Config("library.nativeInvalid".into());
    let mut storage = Box::new(MaybeUninit::<unsafe_yaml::yaml_parser_t>::uninit());
    // SAFETY: storage is properly aligned writable memory. Initialize in place;
    // the parser's input handler holds a self pointer, so never move it out.
    if unsafe { unsafe_yaml::yaml_parser_initialize(storage.as_mut_ptr()) }.fail {
        return Err(invalid());
    }
    let mut scanner = Scanner(storage);
    let parser = scanner.0.as_mut_ptr();
    // SAFETY: text is borrowed for this synchronous scan and outlives scanner.
    // No parser/input pointer escapes, and the initialized allocation is stable.
    unsafe { unsafe_yaml::yaml_parser_set_input_string(parser, text.as_ptr(), text.len() as u64) };
    loop {
        let mut token = MaybeUninit::<unsafe_yaml::yaml_token_t>::uninit();
        // SAFETY: parser is initialized and exclusively accessed here; scan
        // initializes token on success. We never mix scanning with event parsing.
        if unsafe { unsafe_yaml::yaml_parser_scan(parser, token.as_mut_ptr()) }.fail {
            return Err(invalid());
        }
        // SAFETY: scan succeeded. Read only the token kind, then release its
        // buffers exactly once on every successful scan, including early exits.
        let kind = unsafe {
            let kind = (*token.as_ptr()).type_;
            unsafe_yaml::yaml_token_delete(token.as_mut_ptr());
            kind
        };
        match kind {
            unsafe_yaml::YAML_TAG_TOKEN | unsafe_yaml::YAML_TAG_DIRECTIVE_TOKEN => {
                return Err(AppError::Config("library.mcpScopeUnsupported".into()));
            }
            unsafe_yaml::YAML_STREAM_END_TOKEN => return Ok(()),
            _ => {}
        }
    }
}
