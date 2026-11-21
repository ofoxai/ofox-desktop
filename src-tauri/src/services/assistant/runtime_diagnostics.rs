use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(super) const SWITCH: &str = "OFOX_ASSISTANT_RUNTIME_DIAGNOSTICS";
pub(super) const NODE_FLAG: &str = "--ofox-runtime-diagnostics";

/// Accept only complete fixed markers. A dependency can write arbitrarily
/// large or sensitive stderr, so never keep or forward an unrecognized line.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Trace {
    last_stage: Option<&'static str>,
    host_stdin_closed: bool,
    cancel_requested: bool,
    other_stderr: bool,
}

impl Trace {
    fn record(&mut self, raw: &[u8]) {
        const STAGES: &[&str] = &[
            "entry",
            "ready",
            "run_received",
            "sdk_loaded",
            "workspace_ready",
            "provider_dispatch",
            "document_tool_started",
            "document_tool_finished",
            "run_finished",
        ];
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let marker = raw.strip_prefix(b"OFOX_RUNTIME_STAGE:");
        match marker {
            Some(b"host_stdin_closed") => self.host_stdin_closed = true,
            Some(b"cancel_requested") => self.cancel_requested = true,
            Some(stage) => match STAGES.iter().find(|known| known.as_bytes() == stage) {
                Some(known) => self.last_stage = Some(known),
                None => self.other_stderr = true,
            },
            None => self.other_stderr = true,
        }
    }
}

pub(super) async fn collect(mut reader: impl AsyncRead + Unpin) -> Trace {
    let mut trace = Trace::default();
    let mut chunk = [0; 4096];
    let mut line = [0; 80];
    let mut length = 0;
    let mut oversized = false;
    loop {
        let count = match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => {
                trace.other_stderr = true;
                break;
            }
        };
        for &byte in &chunk[..count] {
            if byte == b'\n' {
                if !oversized {
                    trace.record(&line[..length]);
                }
                length = 0;
                oversized = false;
            } else if length < line.len() {
                line[length] = byte;
                length += 1;
            } else {
                oversized = true;
                trace.other_stderr = true;
            }
        }
    }
    trace.other_stderr |= length != 0;
    trace
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retains_last_stage_after_loud_stderr_without_retaining_raw_output() {
        let mut input = b"OFOX_RUNTIME_STAGE:entry\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', 70_000));
        input.extend_from_slice(
            b"OFOX_RUNTIME_STAGE:run_finished\n\
              OFOX_RUNTIME_STAGE:run_received\n\
              diagnostic-secret@example.invalid\n\
              OFOX_RUNTIME_STAGE:workspace_ready\r\n\
              OFOX_RUNTIME_STAGE:host_stdin_closed\n\
              OFOX_RUNTIME_STAGE:workspace_ready:diagnostic-secret\n\
              OFOX_RUNTIME_STAGE:run_finished",
        );
        let trace = collect(input.as_slice()).await;

        assert_eq!(trace.last_stage, Some("workspace_ready"));
        assert!(trace.host_stdin_closed);
        assert!(!trace.cancel_requested);
        assert!(trace.other_stderr);
        assert!(!serde_json::to_string(&trace)
            .unwrap()
            .contains("diagnostic-secret"));
    }

    #[tokio::test]
    async fn accepts_markers_split_between_reads() {
        let (mut writer, reader_pipe) = tokio::io::duplex(3);
        let writing = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            for chunk in b"OFOX_RUNTIME_STAGE:document_tool_finished\n".chunks(3) {
                writer.write_all(chunk).await.unwrap();
            }
        });
        let trace = collect(reader_pipe).await;
        writing.await.unwrap();
        assert_eq!(trace.last_stage, Some("document_tool_finished"));
        assert!(!trace.other_stderr);
    }
}
