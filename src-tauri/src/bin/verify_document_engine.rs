//! Fixed-fixture Windows document isolation acceptance. No user account,
//! gateway credentials, arbitrary scripts, or installed development tools.
use std::path::PathBuf;

fn arguments() -> Result<(PathBuf, PathBuf), &'static str> {
    let mut args = std::env::args_os().skip(1);
    let (mut runtime, mut out) = (None, None);
    while let Some(arg) = args.next() {
        let target = if arg == "--runtime" {
            &mut runtime
        } else if arg == "--out" {
            &mut out
        } else {
            return Err("expected --runtime ABS_RESOURCES --out ABS_REPORT_JSON");
        };
        if target.is_some() {
            return Err("duplicate argument");
        }
        let path = PathBuf::from(args.next().ok_or("missing argument value")?);
        if !path.is_absolute() {
            return Err("acceptance paths must be absolute");
        }
        *target = Some(path);
    }
    Ok((
        runtime.ok_or("missing --runtime")?,
        out.ok_or("missing --out")?,
    ))
}

#[tokio::main]
async fn main() {
    let (runtime, out) = match arguments() {
        Ok(paths) => paths,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    #[cfg(windows)]
    let report = cc_switch_lib::verify_windows_document_engines(&runtime).await;
    #[cfg(not(windows))]
    let report = {
        let _ = runtime;
        serde_json::json!({"schemaVersion":1,"success":false,"diagnostic":"native_windows_required","scope":"native-fixed-fixtures","paidModelCalls":0})
    };
    // New file only: never replace an arbitrary existing user report/config.
    use std::io::Write;
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .and_then(|mut file| {
            file.write_all(
                serde_json::to_string_pretty(&report)
                    .expect("report JSON")
                    .as_bytes(),
            )
        });
    if written.is_err() {
        eprintln!("acceptance_report_write_failed");
        std::process::exit(2);
    }
    println!(
        "{}",
        serde_json::json!({"success":report["success"],"scope":report["scope"]})
    );
    if report["success"] != true {
        std::process::exit(1);
    }
}
