//! Validate the exact JSON/JSONC subset used by the native agents.
use crate::error::AppError;
use serde_json::Value;
fn invalid() -> AppError {
    AppError::Config("library.nativeInvalid".into())
}
pub fn parse(
    source: &str,
    comments: bool,
    trailing_commas: bool,
    bom: bool,
) -> Result<Value, AppError> {
    let source = if bom {
        source.strip_prefix('\u{feff}').unwrap_or(source)
    } else {
        source
    };
    let mut bytes = source.as_bytes().to_vec();
    let mut index = 0;
    let mut quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            quoted = true;
            index += 1;
            continue;
        }
        if comments && byte == b'/' && index + 1 < bytes.len() {
            if bytes[index + 1] == b'/' {
                while index < bytes.len() && bytes[index] != b'\n' && bytes[index] != b'\r' {
                    bytes[index] = b' ';
                    index += 1;
                }
                continue;
            }
            if bytes[index + 1] == b'*' {
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                let mut closed = false;
                while index < bytes.len() {
                    if bytes[index] == b'*' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                        bytes[index] = b' ';
                        bytes[index + 1] = b' ';
                        index += 2;
                        closed = true;
                        break;
                    }
                    if bytes[index] != b'\n' && bytes[index] != b'\r' {
                        bytes[index] = b' ';
                    }
                    index += 1;
                }
                if !closed {
                    return Err(invalid());
                }
                continue;
            }
        }
        index += 1;
    }
    if trailing_commas {
        let mut quoted = false;
        let mut escaped = false;
        for index in 0..bytes.len() {
            let byte = bytes[index];
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            if byte == b'"' {
                quoted = true;
                continue;
            }
            if byte == b',' {
                let next = bytes[index + 1..]
                    .iter()
                    .copied()
                    .find(|byte| !byte.is_ascii_whitespace());
                if matches!(next, Some(b']' | b'}')) {
                    bytes[index] = b' ';
                }
            }
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
/// OpenCode substitutes raw JSON before parsing, including routing key names.
/// Values keep their native reference syntax; dynamic keys cannot be audited safely.
pub fn validate_routing_keys(
    value: &serde_json::Value,
    sections: &[&str],
) -> Result<(), crate::error::AppError> {
    let dynamic = |key: &str| key.contains("{env:") || key.contains("{file:");
    if value
        .as_object()
        .is_some_and(|root| root.keys().any(|key| dynamic(key)))
        || sections.iter().any(|section| {
            value
                .get(section)
                .and_then(serde_json::Value::as_object)
                .is_some_and(|entries| entries.keys().any(|key| dynamic(key)))
        })
    {
        return Err(crate::error::AppError::Config(
            "library.nativeScopeUnsupported".into(),
        ));
    }
    Ok(())
}
