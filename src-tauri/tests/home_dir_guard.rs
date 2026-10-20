//! Guard: the user's home directory is resolved in exactly one place.
//!
//! `crate::config::get_home_dir()` is the only function that may ask the OS for the
//! home directory. It honours `CC_SWITCH_TEST_HOME` and, in unit tests, falls back to
//! a throwaway directory, so no test can read or write the developer's real home.
//! Any other lookup bypasses that sandbox. This test scans `src/**/*.rs` and fails,
//! listing `file:line`, when:
//!
//! * `dirs::home_dir` / `env::home_dir` is used outside `src/config.rs`;
//! * `env::var("HOME")` / `env::var_os("HOME")` is read outside `#[cfg(test)]` code.
//!
//! A deliberate exception carries `// home-guard: allow <reason>` on the same line or
//! on the line directly above it.

use std::fs;
use std::path::{Path, PathBuf};

const ALLOW_MARKER: &str = "home-guard: allow";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Two views of one source file, both the same length as the original with
/// every newline kept, so line numbers and offsets agree.
struct Masked {
    /// Comments blanked; string contents kept (to find `"HOME"`).
    code: Vec<u8>,
    /// Comments, strings and char literals blanked (to count braces safely).
    skeleton: Vec<u8>,
}

fn mask(source: &str) -> Masked {
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut skeleton = bytes.to_vec();
    let blank = |buf: &mut Vec<u8>, i: usize| {
        if buf[i] != b'\n' {
            buf[i] = b' ';
        }
    };

    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();
        if b == b'/' && next == Some(b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                blank(&mut code, i);
                blank(&mut skeleton, i);
                i += 1;
            }
        } else if b == b'/' && next == Some(b'*') {
            let mut depth = 0;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    for k in 0..2 {
                        blank(&mut code, i + k);
                        blank(&mut skeleton, i + k);
                    }
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    for k in 0..2 {
                        blank(&mut code, i + k);
                        blank(&mut skeleton, i + k);
                    }
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut code, i);
                    blank(&mut skeleton, i);
                    i += 1;
                }
            }
        } else if b == b'r'
            && matches!(next, Some(b'"') | Some(b'#'))
            && is_raw_string_start(bytes, i)
        {
            let mut hashes = 0;
            let mut j = i + 1;
            while bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            // bytes[j] is the opening quote.
            let mut end = j + 1;
            loop {
                if end >= bytes.len() {
                    break;
                }
                if bytes[end] == b'"' && (0..hashes).all(|h| bytes.get(end + 1 + h) == Some(&b'#'))
                {
                    end += 1 + hashes;
                    break;
                }
                end += 1;
            }
            for k in j + 1..end.saturating_sub(1 + hashes) {
                blank(&mut skeleton, k);
            }
            i = end;
        } else if b == b'"' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    blank(&mut skeleton, j);
                    j += 1;
                }
                if j < bytes.len() {
                    blank(&mut skeleton, j);
                }
                j += 1;
            }
            i = j + 1;
        } else if b == b'\'' {
            // A char literal ('x', '\n', '{') closes within a few bytes; a
            // lifetime ('a) does not.
            let close = if next == Some(b'\\') {
                bytes[i + 2..]
                    .iter()
                    .take(10)
                    .position(|c| *c == b'\'')
                    .map(|p| i + 2 + p)
            } else if bytes.get(i + 2) == Some(&b'\'') {
                Some(i + 2)
            } else {
                None
            };
            match close {
                Some(close) => {
                    for k in i + 1..close {
                        blank(&mut skeleton, k);
                    }
                    i = close + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    Masked { code, skeleton }
}

fn is_raw_string_start(bytes: &[u8], i: usize) -> bool {
    // `r"..."`, `r#"..."#`; the `r` must not be the tail of an identifier.
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        return false;
    }
    let mut j = i + 1;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    bytes.get(j) == Some(&b'"')
}

/// Byte ranges (half-open) of items gated by `#[cfg(test)]` / `#[cfg(all(test, ..))]`.
fn cfg_test_regions(skeleton: &[u8]) -> Vec<(usize, usize)> {
    let text = String::from_utf8_lossy(skeleton).into_owned();
    let mut regions = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = find_cfg_test(&text[search_from..]) {
        let attr_start = search_from + rel;
        let mut i = attr_start;
        // Skip the attribute itself, then any further attributes.
        loop {
            while i < text.len() && text.as_bytes()[i] != b'[' {
                i += 1;
            }
            i = skip_brackets(text.as_bytes(), i);
            let rest = text[i..].trim_start();
            if rest.starts_with("#[") {
                i = text.len() - rest.len();
            } else {
                break;
            }
        }
        // The item runs to the `;` that ends it, or through its `{ .. }` body.
        let bytes = text.as_bytes();
        let mut end = i;
        let mut depth = 0usize;
        while end < bytes.len() {
            match bytes[end] {
                b';' if depth == 0 => {
                    end += 1;
                    break;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end += 1;
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        regions.push((attr_start, end));
        search_from = end.max(attr_start + 1);
    }
    regions
}

fn find_cfg_test(text: &str) -> Option<usize> {
    ["#[cfg(test)]", "#[cfg(all(test", "#![cfg(test)]"]
        .iter()
        .filter_map(|needle| text.find(needle))
        .min()
}

fn skip_brackets(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

fn is_allowed(source_lines: &[&str], line: usize) -> bool {
    let here = source_lines.get(line - 1).copied().unwrap_or("");
    let above = line
        .checked_sub(2)
        .and_then(|i| source_lines.get(i))
        .copied()
        .unwrap_or("");
    here.contains(ALLOW_MARKER) || above.contains(ALLOW_MARKER)
}

/// Offsets where `needle` occurs in `haystack`.
fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    haystack.match_indices(needle).map(|(i, _)| i).collect()
}

/// `var("HOME")` / `var_os("HOME")` with any spacing inside the parentheses.
fn home_var_reads(code: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    for call in ["var", "var_os"] {
        for start in find_all(code, &format!("{call}(")) {
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue; // e.g. `some_var(`
            }
            let args = code[start + call.len() + 1..].trim_start();
            if args.starts_with("\"HOME\"") {
                hits.push(start);
            }
        }
    }
    hits
}

/// `#[cfg(test)] mod name;` declarations in `source`: those child files are test code.
fn test_module_declarations(source: &str) -> Vec<String> {
    let masked = mask(source);
    cfg_test_regions(&masked.skeleton)
        .into_iter()
        .filter_map(|(start, end)| {
            let item = String::from_utf8_lossy(&masked.skeleton[start..end]).into_owned();
            let after_mod = item.split_once("mod ")?.1;
            let name = after_mod.trim().strip_suffix(';')?.trim();
            name.chars()
                .all(|c| c.is_alphanumeric() || c == '_')
                .then(|| name.to_string())
        })
        .collect()
}

/// Files a `#[cfg(test)] mod name;` in `rel_path` pulls in (`src/`-relative paths).
fn test_module_files(rel_path: &str, names: Vec<String>) -> Vec<String> {
    let path = Path::new(rel_path);
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let base = if matches!(stem, "mod" | "lib" | "main") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    names
        .into_iter()
        .flat_map(|name| {
            [
                base.join(format!("{name}.rs")),
                base.join(&name).join("mod.rs"),
            ]
        })
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect()
}

fn violations_in(rel_path: &str, source: &str) -> Vec<String> {
    violations_in_file(rel_path, source, false)
}

fn violations_in_file(rel_path: &str, source: &str, whole_file_is_test: bool) -> Vec<String> {
    let masked = mask(source);
    let code = String::from_utf8(masked.code).expect("masked code stays utf-8");
    let whole_file_is_test =
        whole_file_is_test || String::from_utf8_lossy(&masked.skeleton).contains("#![cfg(test)]");
    let regions = cfg_test_regions(&masked.skeleton);
    let in_test = |offset: usize| {
        whole_file_is_test || regions.iter().any(|(s, e)| offset >= *s && offset < *e)
    };
    let source_lines: Vec<&str> = source.lines().collect();
    let is_config = rel_path == "config.rs";

    let mut found = Vec::new();
    let mut report = |offset: usize, what: &str| {
        let line = line_of(source, offset);
        if !is_allowed(&source_lines, line) {
            found.push(format!("{rel_path}:{line}: {what}"));
        }
    };

    if !is_config {
        for needle in ["dirs::home_dir", "env::home_dir"] {
            for offset in find_all(&code, needle) {
                report(offset, &format!("`{needle}` outside config.rs"));
            }
        }
        for offset in find_all(&code, "use dirs::") {
            let statement_end = code[offset..].find(';').map_or(code.len(), |p| offset + p);
            if code[offset..statement_end].contains("home_dir") {
                report(offset, "imports `dirs::home_dir` outside config.rs");
            }
        }
    }
    for offset in home_var_reads(&code) {
        if !in_test(offset) {
            report(
                offset,
                "reads the HOME environment variable outside #[cfg(test)]",
            );
        }
    }
    found
}

#[test]
fn home_directory_is_resolved_only_through_get_home_dir() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(
        files.len() > 50,
        "expected to scan the crate sources, found {}",
        files.len()
    );

    let sources: Vec<(String, String)> = files
        .iter()
        .map(|file| {
            let rel = file
                .strip_prefix(&src)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {rel}: {e}"));
            (rel, source)
        })
        .collect();

    // Files pulled in by `#[cfg(test)] mod x;` hold test code only.
    let test_files: Vec<String> = sources
        .iter()
        .flat_map(|(rel, source)| test_module_files(rel, test_module_declarations(source)))
        .collect();
    assert!(
        test_files.iter().any(|f| f.ends_with("tests.rs")),
        "expected to find `#[cfg(test)] mod tests;` declarations"
    );

    let mut violations = Vec::new();
    for (rel, source) in &sources {
        violations.extend(violations_in_file(rel, source, test_files.contains(rel)));
    }

    assert!(
        violations.is_empty(),
        "home directory must come from crate::config::get_home_dir() (it honours the test \
         sandbox). Fix these, or mark a deliberate exception with `// home-guard: allow <reason>`:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_scanner_flags_what_it_should_and_only_that() {
    let flagged = |path: &str, src: &str| violations_in(path, src);

    // Outside config.rs: dirs::home_dir is flagged, in production and in tests alike.
    assert_eq!(
        flagged("a.rs", "fn f() { let _ = dirs::home_dir(); }").len(),
        1
    );
    assert_eq!(
        flagged(
            "a.rs",
            "#[cfg(test)]\nmod t { fn f() { dirs::home_dir(); } }"
        )
        .len(),
        1
    );
    assert_eq!(
        flagged("a.rs", "use dirs::{config_dir, home_dir};").len(),
        1
    );
    assert_eq!(flagged("a.rs", "fn f() { std::env::home_dir(); }").len(), 1);
    // ... but config.rs is the one place that may.
    assert!(flagged("config.rs", "fn f() { dirs::home_dir(); }").is_empty());

    // HOME reads: flagged in production code, fine inside #[cfg(test)] items.
    assert_eq!(
        flagged("a.rs", "fn f() { std::env::var(\"HOME\"); }").len(),
        1
    );
    assert_eq!(
        flagged("a.rs", "fn f() { env::var_os( \"HOME\" ); }").len(),
        1
    );
    assert!(flagged(
        "a.rs",
        "#[cfg(test)]\nmod tests {\n    fn f() { let s = '{'; std::env::var(\"HOME\"); }\n}\n"
    )
    .is_empty());
    assert_eq!(
        flagged(
            "a.rs",
            "#[cfg(test)]\nmod tests { fn f() {} }\nfn prod() { std::env::var(\"HOME\"); }\n"
        )
        .len(),
        1,
        "code after a test module is production code again"
    );
    assert!(flagged(
        "a.rs",
        "#[cfg(test)]\nfn only_in_tests() { std::env::var(\"HOME\"); }\n"
    )
    .is_empty());

    // `#![cfg(test)]` and `#[cfg(test)] mod x;` mark whole files as test code.
    assert!(flagged("a.rs", "#![cfg(test)]\nfn f() { std::env::var(\"HOME\"); }").is_empty());
    assert_eq!(
        test_module_files(
            "services/proxy.rs",
            test_module_declarations("#[cfg(test)]\nmod tests;\nmod other;")
        ),
        ["services/proxy/tests.rs", "services/proxy/tests/mod.rs"]
    );
    assert_eq!(
        test_module_files(
            "lib.rs",
            test_module_declarations("#[cfg(test)] pub(crate) mod fixtures;")
        )[0],
        "fixtures.rs"
    );

    // Other variables, helper names, comments and strings are not reads.
    assert!(flagged(
        "a.rs",
        "fn f() { std::env::var(\"HOMEPATH\"); my_var(\"HOME\"); }"
    )
    .is_empty());
    assert!(flagged("a.rs", "fn f() { get_home_dir(); }").is_empty());
    assert!(flagged(
        "a.rs",
        "// dirs::home_dir() and env::var(\"HOME\")\nfn f() {}"
    )
    .is_empty());

    // The marker allows a line, on the same line or the one above.
    assert!(flagged(
        "a.rs",
        "fn f() { std::env::var(\"HOME\"); } // home-guard: allow shell probe"
    )
    .is_empty());
    assert!(flagged(
        "a.rs",
        "// home-guard: allow shell probe\nfn f() { std::env::var(\"HOME\"); }"
    )
    .is_empty());
}
