//! Fixed, read-only OfficeCLI requests. Only host-selected slide numbers and
//! the two preview filenames can enter the child process's standard input.
use super::runner::{EngineOutput, SVG_OUTPUT_LIMIT};
use serde_json::{json, Value};

pub(super) const MAX_PAGES: usize = 4;

// Includes JSON escaping in the envelope; an oversized envelope falls back to
// the ordinary single-slide path, which retains its original per-slide limit.
pub(super) const OUTPUT_LIMIT: usize = MAX_PAGES * SVG_OUTPUT_LIMIT * 2;
pub(super) struct SvgBatch {
    requests: String,
    page_count: usize,
}

impl SvgBatch {
    pub(super) fn new(file: &str, pages: &[u64]) -> Option<Self> {
        if !matches!(file, "out.pptx" | "preview-compatible.pptx")
            || !(2..=MAX_PAGES).contains(&pages.len())
            || pages
                .iter()
                .any(|&page| !(1..=u64::from(u32::MAX)).contains(&page))
        {
            return None;
        }
        let mut requests = vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"ofox-desktop-preview","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        ];
        for (index, page) in pages.iter().enumerate() {
            let page = page.to_string();
            requests.push(json!({"jsonrpc":"2.0","id":index+2,"method":"tools/call","params":{"name":"officecli","arguments":{"command":["view",file,"svg","--start",page,"--end",page]}}}));
        }
        let requests: String = requests
            .into_iter()
            .map(|request| request.to_string() + "\n")
            .collect();
        (requests.len() <= 4096).then_some(Self {
            requests,
            page_count: pages.len(),
        })
    }

    pub(super) fn timeout(&self, per_page: std::time::Duration) -> std::time::Duration {
        per_page.saturating_mul(self.page_count as u32)
    }

    pub(super) fn input(&self) -> &[u8] {
        self.requests.as_bytes()
    }

    pub(super) fn pictures(&self, output: &EngineOutput) -> Option<Vec<String>> {
        if output.status != Some(0) || output.timed_out || output.stdout.len() >= OUTPUT_LIMIT {
            return None;
        }
        let mut answers = vec![None; self.page_count];
        let mut initialized = false;
        for line in output.stdout.lines().filter(|line| !line.trim().is_empty()) {
            let reply: Value = serde_json::from_str(line).ok()?;
            if reply["jsonrpc"] != "2.0" || reply.get("error").is_some() {
                return None;
            }
            let id = usize::try_from(reply["id"].as_u64()?).ok()?;
            if id == 1 {
                if initialized || !reply["result"]["capabilities"].is_object() {
                    return None;
                }
                initialized = true;
                continue;
            }
            let slot = answers.get_mut(id.checked_sub(2)?)?;
            let result = &reply["result"];
            let content = result["content"].as_array()?;
            if slot.is_some()
                || result["isError"].as_bool() != Some(false)
                || content.len() != 1
                || content[0]["type"] != "text"
            {
                return None;
            }
            let svg = content[0]["text"].as_str()?.trim();
            if svg.len() + 4 >= SVG_OUTPUT_LIMIT || !complete_svg(svg) {
                return None;
            }
            *slot = Some(svg.to_owned());
        }
        initialized.then_some(())?;
        answers.into_iter().collect()
    }
}

// MCP puts command diagnostics and output in the same text block. Accept
// only one complete SVG document so warnings and truncated XML cannot become previews.
fn complete_svg(svg: &str) -> bool {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_str(svg);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut found = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                if tag.attributes().any(|attribute| attribute.is_err()) {
                    return false;
                }
                if depth == 0 {
                    if found || tag.name().as_ref() != b"svg" {
                        return false;
                    }
                    found = true;
                }
                depth += 1;
            }
            Ok(Event::End(_)) if depth > 0 => depth -= 1,
            Ok(Event::Text(text)) if depth > 0 || text.iter().all(u8::is_ascii_whitespace) => {}
            Ok(Event::Decl(_)) if !found => {}
            Ok(Event::Comment(_)) => {}
            Ok(Event::CData(_) | Event::GeneralRef(_)) if depth > 0 => {}
            Ok(Event::Eof) => return found && depth == 0,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result(id: usize, svg: &str) -> Value {
        json!({"jsonrpc":"2.0","id":id,"result":{"isError":false,"content":[{"type":"text","text":svg}]}})
    }
    fn output(replies: &[Value]) -> EngineOutput {
        EngineOutput {
            status: Some(0),
            timed_out: false,
            stderr: String::new(),
            stdout: replies.iter().map(|r| r.to_string() + "\n").collect(),
        }
    }

    #[test]
    fn sparse_reordered_replies_retain_their_slide_and_reject_invalid_envelopes() {
        let batch = SvgBatch::new("preview-compatible.pptx", &[7, 3]).unwrap();
        assert!(SvgBatch::new("arbitrary.pptx", &[1, 2]).is_none());
        assert_eq!(
            batch.timeout(std::time::Duration::from_secs(60)).as_secs(),
            120
        );
        let requests: Vec<Value> = std::str::from_utf8(batch.input())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            requests[2]["params"]["arguments"]["command"],
            json!([
                "view",
                "preview-compatible.pptx",
                "svg",
                "--start",
                "7",
                "--end",
                "7"
            ])
        );
        assert_eq!(requests[3]["params"]["arguments"]["command"][4], "3");
        let init = json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}});
        let seven = result(2, "<svg>seven</svg>");
        let three = result(3, "<svg>three</svg>");
        assert_eq!(
            batch
                .pictures(&output(&[init.clone(), three.clone(), seven.clone()]))
                .unwrap(),
            ["<svg>seven</svg>", "<svg>three</svg>"]
        );
        for bad in [
            result(2, "<svg><path/>"),
            result(2, "<svg broken/>"),
            result(2, "<svg/>\n[stderr] <error/>"),
            result(2, "<svg/><svg/>"),
            json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"<svg/>"}]}}),
            three.clone(),
        ] {
            assert!(batch
                .pictures(&output(&[init.clone(), three.clone(), bad]))
                .is_none());
        }
    }

    #[test]
    fn pinned_officecli_renders_two_distinct_synthetic_slides_in_one_process() {
        use std::process::{Command, Stdio};
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime");
        let exe = base
            .join("resources/engines/officecli")
            .join(format!("officecli{}", std::env::consts::EXE_SUFFIX));
        if !exe.exists() && cfg!(target_os = "linux") {
            return;
        }
        let fixture = tempfile::tempdir().unwrap();
        let deck = fixture.path().join("out.pptx");
        std::fs::copy(base.join("test/fixtures/office/chinese.pptx"), &deck).unwrap();
        let original = std::fs::read(&deck).unwrap();
        let command = || {
            let mut command = Command::new(&exe);
            command
                .current_dir(fixture.path())
                .env("OFFICECLI_SKIP_UPDATE", "1")
                .env("OFFICECLI_NO_AUTO_RESIDENT", "1")
                .env("HOME", fixture.path())
                .env("USERPROFILE", fixture.path())
                .env("LOCALAPPDATA", fixture.path())
                .env("APPDATA", fixture.path())
                .env("TEMP", fixture.path())
                .env("TMP", fixture.path());
            command
        };
        let batch = SvgBatch::new("out.pptx", &[2, 1]).unwrap();
        let input = fixture.path().join("fixed-requests.jsonl");
        std::fs::write(&input, batch.input()).unwrap();
        let result = command()
            .arg("mcp")
            .stdin(Stdio::from(std::fs::File::open(input).unwrap()))
            .output()
            .unwrap();
        let pictures = batch
            .pictures(&EngineOutput {
                status: result.status.code(),
                timed_out: false,
                stdout: String::from_utf8(result.stdout).unwrap(),
                stderr: String::from_utf8(result.stderr).unwrap(),
            })
            .unwrap();
        for (svg, page) in pictures.iter().zip(["2", "1"]) {
            let one = command()
                .args(["view", "out.pptx", "svg", "--start", page, "--end", page])
                .output()
                .unwrap();
            assert!(one.status.success());
            assert_eq!(
                svg.replace("\r\n", "\n"),
                String::from_utf8(one.stdout)
                    .unwrap()
                    .trim()
                    .replace("\r\n", "\n")
            );
        }
        assert_ne!(pictures[0], pictures[1]);
        assert_eq!(std::fs::read(deck).unwrap(), original);
    }
}
