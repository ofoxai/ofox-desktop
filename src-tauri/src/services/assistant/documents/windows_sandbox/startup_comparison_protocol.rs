//! Fixed test receipts, never a runtime admission policy.
use serde_json::Value;

pub(super) fn eligible(error: &str) -> bool {
    let Some((_, data)) = error.split_once(":probe ") else {
        return false;
    };
    if data.len() > 65536 {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    let e = &value["worker"]["evidence"];
    let d = &e["loaderDiagnostic"];
    let Some(before) = d["beforeFlags"].as_u64().filter(|v| *v <= u32::MAX as u64) else {
        return false;
    };
    e["processStatus"] == 0xc0000142u32
        && e["resumeResult"] == 1
        && d["diagnosticOnly"] == true
        && d["configConfirmed"] == true
        && d["afterFlags"].as_u64() == Some(before | 2)
        && d["preAttachSuspendPrevious"] == 1
        && d["preAttachPairedResumePrevious"] == 2
        && d["suspendPrevious"] == 1
        && d["pairedResumePrevious"] == 2
        && d["hostResumePrevious"] == 1
        && d["debuggerClosed"] == true
}

fn verified(row: &Value, count: u64) -> bool {
    row["completed"] == true
        && row["tokenVerified"] == true
        && row["cleanupConfirmed"] == true
        && row["resumePrevious"] == 1
        && row["timedOut"] == false
        && row["capabilityCount"].as_u64() == Some(count)
        && row["lpacAccessMatched"] == true
        && row["integrityRid"].as_u64().is_some_and(|v| v <= 4096)
        && row["childPolicy"].as_u64().is_some_and(|v| v & 1 == 1)
        && row["imagePolicy"].as_u64().is_some_and(|v| v & 3 == 3)
        && row["capabilityAttributes"]
            .as_array()
            .is_some_and(|v| v.len() == count as usize && v.iter().all(|v| v == 4))
        && row["stderrBytes"] == 0
}
pub(super) fn baseline(row: &Value) -> bool {
    verified(row, 1)
        && row["extraRegistryRead"] == false
        && row["exitStatus"] == 0xc0000142u32
        && row["markerMatched"] == false
        && row["stdoutBytes"] == 0
}
pub(super) fn supported(rows: &[Value]) -> bool {
    rows.len() == 3
        && baseline(&rows[0])
        && baseline(&rows[2])
        && verified(&rows[1], 2)
        && rows[1]["extraRegistryRead"] == true
        && rows[1]["exitStatus"] == 0
        && rows[1]["markerMatched"] == true
        && rows[1]["stdoutBytes"] == "OFOX_SYSTEM_INIT_V1".len()
        && [1, 2].into_iter().all(|index| {
            ["childPolicy", "imagePolicy", "integrityRid"]
                .into_iter()
                .all(|key| rows[index][key] == rows[0][key])
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(extra: bool) -> Value {
        serde_json::json!({"completed":true,"tokenVerified":true,"cleanupConfirmed":true,"resumePrevious":1,
            "timedOut":false,"capabilityCount":if extra{2}else{1},"lpacAccessMatched":true,"childPolicy":1,"imagePolicy":7,"integrityRid":4096,
            "capabilityAttributes":if extra{vec![4,4]}else{vec![4]},"stderrBytes":0,"extraRegistryRead":extra,
            "exitStatus":if extra{0}else{0xc0000142u32},"markerMatched":extra,"stdoutBytes":if extra{19}else{0}})
    }
    #[test]
    fn all_three_actual_verified_results_are_required() {
        let rows = [row(false), row(true), row(false)];
        assert!(supported(&rows));
        for length in 0..3 {
            assert!(!supported(&rows[..length]));
        }
        for index in 0..3 {
            for field in [
                "completed",
                "tokenVerified",
                "cleanupConfirmed",
                "resumePrevious",
                "timedOut",
                "capabilityCount",
                "lpacAccessMatched",
                "childPolicy",
                "imagePolicy",
                "integrityRid",
                "capabilityAttributes",
                "stderrBytes",
                "extraRegistryRead",
                "exitStatus",
                "markerMatched",
                "stdoutBytes",
            ] {
                let mut bad = rows.clone();
                bad[index].as_object_mut().unwrap().remove(field);
                assert!(!supported(&bad), "{index}:{field}");
            }
        }
        let mut warm = rows.clone();
        warm[2] = row(true);
        assert!(!supported(&warm));
        for (field, value) in [("childPolicy", 3), ("imagePolicy", 3), ("integrityRid", 0)] {
            let mut confounded = rows.clone();
            confounded[1][field] = serde_json::json!(value);
            assert!(!supported(&confounded));
        }
    }
    #[test]
    fn incomplete_or_wrong_original_failure_never_arms_comparison() {
        let mut e = serde_json::json!({"processStatus":0xc0000142u32,"resumeResult":1,"loaderDiagnostic":{
            "diagnosticOnly":true,"configConfirmed":true,"beforeFlags":0,"afterFlags":2,"preAttachSuspendPrevious":1,
            "preAttachPairedResumePrevious":2,"suspendPrevious":1,"pairedResumePrevious":2,"hostResumePrevious":1,"debuggerClosed":true}});
        let message = |e: &Value| {
            format!(
                "unavailable:probe {}",
                serde_json::json!({"worker":{"evidence":e}})
            )
        };
        assert!(eligible(&message(&e)));
        for field in [
            "diagnosticOnly",
            "configConfirmed",
            "beforeFlags",
            "afterFlags",
            "preAttachSuspendPrevious",
            "preAttachPairedResumePrevious",
            "suspendPrevious",
            "pairedResumePrevious",
            "hostResumePrevious",
            "debuggerClosed",
        ] {
            let mut bad = e.clone();
            bad["loaderDiagnostic"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(!eligible(&message(&bad)), "{field}");
        }
        e["processStatus"] = serde_json::json!(0);
        assert!(!eligible(&message(&e)));
        assert!(!eligible("unavailable"));
        assert!(!eligible("unavailable:probe broken"));
    }
}
