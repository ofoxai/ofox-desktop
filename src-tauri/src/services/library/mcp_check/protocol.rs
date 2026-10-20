use super::Failure;
use serde_json::{json, Value};
use std::future::Future;

pub(super) const MODERN: &str = "2026-07-28";
pub(super) const LEGACY: &str = "2025-06-18";
pub(super) trait Rpc {
    fn call(
        &mut self,
        method: &str,
        params: Value,
        version: &str,
    ) -> impl Future<Output = Result<Value, Failure>> + Send;
    fn notify(
        &mut self,
        method: &str,
        params: Value,
        version: &str,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
}
fn modern_params() -> Value {
    json!({"_meta":{
        "io.modelcontextprotocol/protocolVersion":MODERN,
        "io.modelcontextprotocol/clientCapabilities":{},
        "io.modelcontextprotocol/clientInfo":{"name":"ofox-mcp-check","version":"1.0.0"}
    }})
}
fn cache_result(result: &Value) -> Result<(), Failure> {
    if result["resultType"] != "complete"
        || result["ttlMs"].as_u64().is_none()
        || !["private", "public"].contains(&result["cacheScope"].as_str().unwrap_or_default())
    {
        return Err(Failure::Invalid);
    }
    Ok(())
}
pub(super) async fn inspect(rpc: &mut impl Rpc, legacy_only: bool) -> Result<Vec<Value>, Failure> {
    let discovery = if legacy_only {
        Err(Failure::Rpc(-32601))
    } else {
        rpc.call("server/discover", modern_params(), MODERN).await
    };
    let (modern, version, capabilities) = match discovery {
        Ok(result) => {
            cache_result(&result)?;
            if !result["supportedVersions"]
                .as_array()
                .is_some_and(|versions| versions.iter().any(|v| v == MODERN))
            {
                return Err(Failure::UnsupportedVersion);
            }
            (true, MODERN.to_owned(), result["capabilities"].clone())
        }
        Err(Failure::Rpc(code)) if ![-32020, -32021, -32022].contains(&code) => legacy(rpc).await?,
        Err(Failure::DiscoveryTimeout | Failure::LegacyRequired) => legacy(rpc).await?,
        Err(Failure::Rpc(-32022)) => return Err(Failure::UnsupportedVersion),
        Err(error) => return Err(error),
    };
    list(rpc, modern, version, capabilities).await
}
async fn legacy(rpc: &mut impl Rpc) -> Result<(bool, String, Value), Failure> {
    let result=rpc.call("initialize",json!({"protocolVersion":LEGACY,"capabilities":{},"clientInfo":{"name":"ofox-mcp-check","version":"1.0.0"}}),LEGACY).await?;
    let version = result["protocolVersion"].as_str().ok_or(Failure::Invalid)?;
    if !["2024-11-05", "2025-03-26", LEGACY, "2025-11-25"].contains(&version) {
        return Err(Failure::UnsupportedVersion);
    }
    if !result["serverInfo"].is_object() {
        return Err(Failure::Invalid);
    }
    rpc.notify("notifications/initialized", json!({}), version)
        .await?;
    Ok((false, version.to_owned(), result["capabilities"].clone()))
}
async fn list(
    rpc: &mut impl Rpc,
    modern: bool,
    version: String,
    capabilities: Value,
) -> Result<Vec<Value>, Failure> {
    if !capabilities.is_object() {
        return Err(Failure::Invalid);
    }
    if capabilities.get("tools").is_none() {
        return Ok(vec![]);
    }
    if !capabilities["tools"].is_object() {
        return Err(Failure::Invalid);
    }
    let mut all = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut cursor = None;
    for _ in 0..10 {
        let mut params = if modern { modern_params() } else { json!({}) };
        if let Some(cursor) = &cursor {
            params["cursor"] = json!(cursor)
        }
        let page = rpc.call("tools/list", params, &version).await?;
        if modern {
            cache_result(&page)?;
        }
        for tool in page["tools"].as_array().ok_or(Failure::Invalid)? {
            let name = tool["name"]
                .as_str()
                .filter(|name| !name.is_empty() && name.len() <= 256)
                .ok_or(Failure::Invalid)?;
            if !tool["inputSchema"].is_object() || !names.insert(name.to_owned()) {
                return Err(Failure::Invalid);
            }
            if all.len() >= 200 {
                return Err(Failure::Limit);
            }
            all.push(tool.clone());
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => return Ok(all),
            Some(Value::String(next)) if next.len() <= 4096 => {
                if !seen.insert(next.clone()) {
                    return Err(Failure::Limit);
                }
                cursor = Some(next.clone());
            }
            _ => return Err(Failure::Invalid),
        }
    }
    Err(Failure::Limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fixture {
        replies: VecDeque<Result<Value, Failure>>,
        sent: Vec<(String, Value, String)>,
    }
    impl Rpc for Fixture {
        async fn call(
            &mut self,
            method: &str,
            params: Value,
            version: &str,
        ) -> Result<Value, Failure> {
            self.sent.push((method.into(), params, version.into()));
            self.replies.pop_front().expect("unexpected RPC")
        }
        async fn notify(
            &mut self,
            method: &str,
            params: Value,
            version: &str,
        ) -> Result<(), Failure> {
            self.sent.push((method.into(), params, version.into()));
            Ok(())
        }
    }
    fn modern() -> Value {
        json!({"resultType":"complete","supportedVersions":[MODERN],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
    }
    fn page(tools: Value, cursor: Option<&str>) -> Value {
        let mut result =
            json!({"resultType":"complete","tools":tools,"ttlMs":0,"cacheScope":"private"});
        if let Some(cursor) = cursor {
            result["nextCursor"] = json!(cursor)
        }
        result
    }
    #[tokio::test]
    async fn empty_opaque_cursor_and_generic_legacy_preinit_error_are_valid() {
        let mut rpc = Fixture {
            replies: [
                Ok(modern()),
                Ok(page(json!([]), Some(""))),
                Ok(page(json!([{"name":"valid","inputSchema":{}}]), None)),
            ]
            .into(),
            sent: vec![],
        };
        assert_eq!(inspect(&mut rpc, false).await.unwrap().len(), 1);
        assert_eq!(rpc.sent[2].1["cursor"], "");
        let mut rpc=Fixture{replies:[Err(Failure::Rpc(-32000)),Ok(json!({"protocolVersion":LEGACY,"capabilities":{},"serverInfo":{"name":"fixture"}}))].into(),sent:vec![]};
        assert_eq!(inspect(&mut rpc, false).await.unwrap().len(), 0);
        assert_eq!(rpc.sent[1].0, "initialize");
    }
    #[tokio::test]
    async fn modern_discovery_lists_every_page_including_empty_pages_without_calling_tools() {
        let mut rpc = Fixture {
            replies: [
                Ok(modern()),
                Ok(page(json!([]), Some("next"))),
                Ok(page(
                    json!([{"name":"fixture-tool","inputSchema":{"type":"object"}}]),
                    None,
                )),
            ]
            .into(),
            sent: vec![],
        };
        let tools = inspect(&mut rpc, false).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(
            rpc.sent
                .iter()
                .map(|request| request.0.as_str())
                .collect::<Vec<_>>(),
            ["server/discover", "tools/list", "tools/list"]
        );
        assert_eq!(
            rpc.sent[0].1["_meta"]["io.modelcontextprotocol/protocolVersion"],
            MODERN
        );
        assert_eq!(rpc.sent[2].1["cursor"], "next");
    }
    #[tokio::test]
    async fn only_discovery_compatibility_errors_may_fall_back_to_legacy_initialization() {
        let mut rpc=Fixture{replies:[Err(Failure::Rpc(-32601)),Ok(json!({"protocolVersion":LEGACY,"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})),Ok(json!({"tools":[{"name":"legacy","inputSchema":{"type":"object"}}]}))].into(),sent:vec![]};
        assert_eq!(inspect(&mut rpc, false).await.unwrap().len(), 1);
        assert_eq!(
            rpc.sent.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            [
                "server/discover",
                "initialize",
                "notifications/initialized",
                "tools/list"
            ]
        );
        for error in [
            Failure::Cancelled,
            Failure::Timeout,
            Failure::Invalid,
            Failure::Rpc(-32021),
        ] {
            let mut rpc = Fixture {
                replies: [Err(error)].into(),
                sent: vec![],
            };
            assert!(inspect(&mut rpc, false).await.is_err());
            assert_eq!(rpc.sent.len(), 1);
        }
    }
    #[tokio::test]
    async fn modern_schema_errors_and_repeated_pagination_are_bounded_failures() {
        let mut invalid = modern();
        invalid["resultType"] = json!("unknown");
        let mut rpc = Fixture {
            replies: [Ok(invalid)].into(),
            sent: vec![],
        };
        assert_eq!(inspect(&mut rpc, false).await, Err(Failure::Invalid));
        let mut rpc = Fixture {
            replies: [
                Ok(modern()),
                Ok(page(json!([]), Some("same"))),
                Ok(page(json!([]), Some("same"))),
            ]
            .into(),
            sent: vec![],
        };
        assert_eq!(inspect(&mut rpc, false).await, Err(Failure::Limit));
    }
}
