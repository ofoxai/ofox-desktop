//! The host's side of web search (assistant-runtime README, "Contract for
//! the search host"): the pages a run may read, the tool progress the
//! renderer is shown, and the sources a search returned.
use super::{host::PROTOCOL, types::*};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// The runtime takes at most this many pages a run may read...
pub(super) const MAX_ALLOWED_WEB_URLS: usize = 500;
/// ... each at most this long.
const MAX_WEB_URL_CHARS: usize = 2048;
/// The search server's own limits on what it returns.
const MAX_TITLE_CHARS: usize = 200;
const MAX_EXCERPT_CHARS: usize = 300;
/// The sources one reply keeps (20 searches of 25 results at most, mostly
/// the same pages).
const MAX_SOURCES: usize = 200;

/// Names that never lead to a public page (the search server refuses them
/// too), as the last label or the whole name.
const LOCAL_NAMES: [&str; 5] = ["localhost", "local", "internal", "invalid", "test"];

/// A public IPv4 address, as the search server reads it (`isPublicAddress`).
fn is_public_v4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99)))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}

/// A global unicast IPv6 address outside the transition and documentation
/// ranges, as the search server reads it.
fn is_public_v6(ip: std::net::Ipv6Addr) -> bool {
    let [first, second, ..] = ip.segments();
    (0x2000..0x4000).contains(&first)
        && !(first == 0x2001 && (second < 0x200 || second == 0xdb8))
        && ![0x2002, 0x3ffe, 0x3fff].contains(&first)
}

/// A host a public page can be on: a dotted name that is not local or
/// reserved, or a public address.
pub(super) fn is_public_host(host: url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.');
            let last = name.rsplit('.').next().unwrap_or(name);
            name.contains('.') && !LOCAL_NAMES.contains(&last)
        }
        url::Host::Ipv4(ip) => is_public_v4(ip),
        url::Host::Ipv6(ip) => is_public_v6(ip),
    }
}

/// `value` as `web_fetch` compares pages: an HTTPS URL of a public host (see
/// [`is_public_host`]), without credentials, without its fragment and at
/// most [`MAX_WEB_URL_CHARS`] long. Scheme and host come out lowercase.
pub(super) fn web_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !url.host().is_some_and(is_public_host)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    let url = String::from(url);
    (url.len() <= MAX_WEB_URL_CHARS).then_some(url)
}

/// Ends a link written in text: spaces, quotes and brackets that cannot be
/// part of it, and the CJK punctuation that follows one without a space.
fn ends_link(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '<' | '>' | '"' | '\'' | '`' | '[' | ']' | '{' | '}' | '|' | '\\' | '^'
        )
        || matches!(
            c,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '！'
                | '？'
                | '（'
                | '）'
                | '【'
                | '】'
                | '「'
                | '」'
                | '『'
                | '』'
                | '《'
                | '》'
                | '“'
                | '”'
                | '‘'
                | '’'
        )
}

/// The HTTPS links written in `text` (see [`web_url`]), in order, whatever
/// the case of their scheme. Sentence punctuation after a link, and a closing
/// parenthesis it did not open (a Markdown link's), are not part of it.
pub(super) fn https_links(text: &str) -> Vec<String> {
    let mut links = vec![];
    // Same byte offsets as `text`: only ASCII letters change.
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("https://") {
        let tail = &text[from + found..];
        let end = tail.find(ends_link).unwrap_or(tail.len());
        let mut link = &tail[..end];
        loop {
            let mut trimmed = link.trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_']);
            if trimmed.ends_with(')') && trimmed.matches(')').count() > trimmed.matches('(').count()
            {
                trimmed = &trimmed[..trimmed.len() - 1];
            }
            if trimmed.len() == link.len() {
                break;
            }
            link = trimmed;
        }
        links.extend(web_url(link));
        from += found + end;
    }
    links
}

/// The pages `web_fetch` may read beyond what the run's own searches return:
/// the links in `prompt`, then in the questions and the sources of `earlier`
/// (newest first). The prompt's and the newest are kept when there are more
/// than the runtime takes; the list is sorted.
pub(super) fn allowed_web_urls<'a>(
    prompt: &str,
    earlier: impl Iterator<Item = &'a Message>,
) -> Vec<String> {
    let earlier = earlier.flat_map(|m| {
        let asked = if m.role == "user" {
            https_links(&m.content)
        } else {
            vec![]
        };
        let sources = m
            .sources
            .iter()
            .filter_map(|source| source["url"].as_str().and_then(web_url));
        asked.into_iter().chain(sources)
    });
    let mut seen = HashSet::new();
    let mut urls: Vec<String> = https_links(prompt)
        .into_iter()
        .chain(earlier)
        .filter(|url| seen.insert(url.clone()))
        .take(MAX_ALLOWED_WEB_URLS)
        .collect();
    urls.sort();
    urls
}

/// A tool event's tool and call; anything else is a protocol error.
fn tool_call(data: &Value) -> Result<(&str, &str), String> {
    match (data["name"].as_str(), data["toolCallId"].as_str()) {
        (Some(name), Some(call)) if !name.is_empty() && !call.is_empty() => Ok((name, call)),
        _ => Err(PROTOCOL.into()),
    }
}

/// The `tool_start` the renderer is shown: the tool, its call and the one
/// argument worth showing (the query, the page, the attachment).
pub(super) fn tool_start_event(data: &Value) -> Result<Value, String> {
    let (name, call) = tool_call(data)?;
    let shown = match name {
        "web_search" => Some("query"),
        "web_fetch" => Some("url"),
        "read_attachment" => Some("id"),
        _ => None,
    };
    let mut args = Map::new();
    if let Some((key, value)) =
        shown.and_then(|key| Some((key, data["args"][key].as_str()?.to_owned())))
    {
        args.insert(key.into(), json!(value));
    }
    Ok(json!({"name": name, "toolCallId": call, "args": args}))
}

/// A tool failure's code: the runtime's `details.code`, else the head of its
/// `<code>: <message>`; `None` for free text.
fn failure_code(data: &Value) -> Option<String> {
    let is_code = |code: &&str| {
        (1..=64).contains(&code.len())
            && code.starts_with(|c: char| c.is_ascii_lowercase())
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    data["details"]["code"]
        .as_str()
        .filter(is_code)
        .or_else(|| {
            data["message"]
                .as_str()
                .and_then(|message| message.split_once(':'))
                .map(|(code, _)| code)
                .filter(is_code)
        })
        .map(str::to_owned)
}

/// The `tool_end` the renderer is shown: whether the call succeeded and, if
/// not, its code. The model's error text is not shown.
pub(super) fn tool_end_event(data: &Value) -> Result<Value, String> {
    let (name, call) = tool_call(data)?;
    let success = data["success"].as_bool().ok_or(PROTOCOL)?;
    let code = if success { None } else { failure_code(data) };
    Ok(json!({"name": name, "toolCallId": call, "success": success, "code": code}))
}

fn clip(text: &str, chars: usize) -> String {
    text.trim().chars().take(chars).collect()
}

/// The sources a successful `web_search` returned, as a reply keeps them:
/// HTTPS pages only (see [`web_url`]), `{id, title, url, excerpt}`.
pub(super) fn search_sources(run_id: &str, details: &Value) -> Vec<Value> {
    let search = id();
    details["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|source| {
            let url = web_url(source["url"].as_str()?)?;
            let title = match source["title"].as_str().map(|t| clip(t, MAX_TITLE_CHARS)) {
                Some(title) if !title.is_empty() => title,
                _ => url.clone(),
            };
            let excerpt = clip(
                source["snippet"].as_str().unwrap_or_default(),
                MAX_EXCERPT_CHARS,
            );
            Some((url, title, excerpt))
        })
        .enumerate()
        .map(|(index, (url, title, excerpt))| {
            json!({
                "id": format!("{run_id}-{search}-{}", index + 1),
                "title": title, "url": url, "excerpt": excerpt,
            })
        })
        .collect()
}

/// Add the sources `message` does not have yet (by page), up to
/// [`MAX_SOURCES`]; whether any was added.
pub(super) fn add_sources(message: &mut Message, found: Vec<Value>) -> bool {
    let mut known: HashSet<String> = message
        .sources
        .iter()
        .filter_map(|source| source["url"].as_str().map(str::to_owned))
        .collect();
    let before = message.sources.len();
    for source in found {
        if message.sources.len() >= MAX_SOURCES {
            break;
        }
        if source["url"]
            .as_str()
            .is_some_and(|url| known.insert(url.to_owned()))
        {
            message.sources.push(source);
        }
    }
    message.sources.len() > before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_the_https_pages_web_fetch_could_read() {
        let text = concat!(
            "See https://example.com/a, and [docs](https://docs.example.com/guide).\n",
            "Also <https://example.com/b?x=1#part> and https://en.wikipedia.org/wiki/Rust_(programming_language).\n",
            "看看https://example.cn/新闻，还有 https://example.cn/page。\n",
            "Not http://plain.example.com, not https://user:pw@secret.example.com/ ",
            "nor ftp://files.example.com or **https://bold.example.com/**",
        );
        assert_eq!(
            https_links(text),
            [
                "https://example.com/a",
                "https://docs.example.com/guide",
                "https://example.com/b?x=1",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "https://example.cn/%E6%96%B0%E9%97%BB",
                "https://example.cn/page",
                "https://bold.example.com/",
            ]
        );
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert!(https_links(&long).is_empty());
        assert!(https_links("https://").is_empty());
        // Any case of the scheme, normalized.
        assert_eq!(
            https_links("HTTPS://Example.COM/Path and Https://pi.dev"),
            ["https://example.com/Path", "https://pi.dev/"]
        );
    }

    #[test]
    fn local_and_reserved_hosts_are_never_pages_a_run_may_read() {
        for refused in [
            "https://localhost/",
            "https://LOCALHOST./x",
            "https://api.localhost/",
            "https://printer.local/",
            "https://metadata.internal/",
            "https://page.example.invalid/",
            "https://site.test/",
            "https://intranet/",
            "https://127.0.0.1/",
            "https://10.1.2.3/",
            "https://172.20.0.1/",
            "https://192.168.1.1/",
            "https://169.254.169.254/latest",
            "https://100.64.0.1/",
            "https://198.18.0.1/",
            "https://0.0.0.0/",
            "https://224.0.0.1/",
            "https://[::1]/",
            "https://[fe80::1]/",
            "https://[fc00::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[2001:db8::1]/",
        ] {
            assert_eq!(web_url(refused), None, "{refused}");
        }
        for allowed in [
            "https://8.8.8.8/",
            "https://[2606:4700::1111]/",
            "https://example.com:8443/doc",
            "https://local.example.com/",
            "https://testing.example.org/",
        ] {
            assert!(web_url(allowed).is_some(), "{allowed}");
        }
    }

    #[test]
    fn tool_progress_shows_the_tool_and_one_argument_and_never_the_error_text() {
        let start = |name: &str, args: Value| {
            tool_start_event(&json!({"name": name, "toolCallId": "call-1", "args": args}))
        };
        assert_eq!(
            start("web_search", json!({"query": "Pi SDK", "extra": 1})).unwrap(),
            json!({"name": "web_search", "toolCallId": "call-1", "args": {"query": "Pi SDK"}})
        );
        assert_eq!(
            start("web_fetch", json!({"url": "https://pi.dev/"})).unwrap()["args"],
            json!({"url": "https://pi.dev/"})
        );
        assert_eq!(
            start("read_attachment", json!({"id": "a1", "offset": 20000})).unwrap()["args"],
            json!({"id": "a1"})
        );
        assert_eq!(
            start("other_tool", json!({"secret": "x"})).unwrap()["args"],
            json!({})
        );
        assert_eq!(
            tool_start_event(&json!({"toolCallId": "call-1"})).unwrap_err(),
            PROTOCOL
        );

        let end = |data: Value| tool_end_event(&data).unwrap();
        assert_eq!(
            end(json!({"name": "web_search", "toolCallId": "c", "success": true, "details": {}})),
            json!({"name": "web_search", "toolCallId": "c", "success": true, "code": null})
        );
        for (data, code) in [
            (
                json!({"name": "web_search", "toolCallId": "c", "success": false,
                       "message": "search_failed: Rate limited",
                       "details": {"code": "search_failed", "status": 429}}),
                json!("search_failed"),
            ),
            (
                json!({"name": "web_search", "toolCallId": "c", "success": false,
                       "message": "search_outcome_unknown: may have run"}),
                json!("search_outcome_unknown"),
            ),
            (
                json!({"name": "read_attachment", "toolCallId": "c", "success": false,
                       "message": "assistant_vision_model_required: choose a vision model"}),
                json!("assistant_vision_model_required"),
            ),
            (
                json!({"name": "read_attachment", "toolCallId": "c", "success": false,
                       "message": "page applies only to PDF attachments"}),
                Value::Null,
            ),
        ] {
            let shown = end(data);
            assert_eq!(shown["code"], code);
            assert!(shown.get("message").is_none());
        }
        assert_eq!(
            tool_end_event(&json!({"name": "web_search", "toolCallId": "c"})).unwrap_err(),
            PROTOCOL
        );
    }

    #[test]
    fn a_reply_keeps_only_https_sources_once_each() {
        let details = json!({
            "usage": {"searchCalls": 1, "inputTokens": 1, "outputTokens": 1, "model": "m"},
            "sources": [
                {"title": " Pi SDK ", "url": "https://pi.dev/docs#sdk", "snippet": "Build agents"},
                {"title": "Plain", "url": "http://plain.example.com/"},
                {"title": "Login", "url": "https://user:pw@example.com/"},
                {"title": "", "url": "https://example.com/untitled"},
                {"title": "x".repeat(500), "url": "https://example.com/long", "snippet": "y".repeat(500)},
                {"title": "No URL"},
                "not a source",
            ],
        });
        let found = search_sources("run-1", &details);
        let pages: Vec<_> = found.iter().map(|s| s["url"].as_str().unwrap()).collect();
        assert_eq!(
            pages,
            [
                "https://pi.dev/docs",
                "https://example.com/untitled",
                "https://example.com/long",
            ]
        );
        assert_eq!(found[0]["title"], "Pi SDK");
        assert_eq!(found[0]["excerpt"], "Build agents");
        assert_eq!(found[1]["title"], "https://example.com/untitled");
        assert_eq!(found[1]["excerpt"], "");
        assert_eq!(found[2]["title"].as_str().unwrap().chars().count(), 200);
        assert_eq!(found[2]["excerpt"].as_str().unwrap().chars().count(), 300);
        let ids: HashSet<_> = found.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().all(|id| id.starts_with("run-1-")));
        assert!(search_sources("run-1", &json!({"sources": null})).is_empty());

        let mut reply: Message = serde_json::from_value(json!({
            "id": id(), "role": "assistant", "content": "", "createdAt": 1,
            "status": "streaming",
        }))
        .unwrap();
        assert!(add_sources(&mut reply, found.clone()));
        // The same pages again (a repeated query): nothing new.
        assert!(!add_sources(&mut reply, search_sources("run-1", &details)));
        assert_eq!(reply.sources, found);
        let many: Vec<Value> = (0..300)
            .map(|i| json!({"id": i, "url": format!("https://example.com/{i}")}))
            .collect();
        assert!(add_sources(&mut reply, many));
        assert_eq!(reply.sources.len(), MAX_SOURCES);
    }
}
