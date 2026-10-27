// The app's proxy (OFOX_PROXY_URL / OFOX_NO_PROXY), used the way the
// assistant runtime uses it for model requests: undici (pinned, the version
// the runtime ships) tunnels every request through an HTTP(S) or SOCKS5 proxy
// unless the target matches NO_PROXY.
import { EnvHttpProxyAgent, fetch as undiciFetch } from "undici";

const DEFAULT_PORTS = { "http:": 80, "https:": 443 };

/** fetch through the proxy; NO_PROXY targets connect directly. */
export function proxiedFetch(proxy) {
  const dispatcher = new EnvHttpProxyAgent({
    httpProxy: proxy.url,
    httpsProxy: proxy.url,
    noProxy: proxy.noProxy ?? "",
    proxyTunnel: true,
  });
  return (input, init) => undiciFetch(input, { ...init, dispatcher });
}

function noProxyEntries(noProxy) {
  return noProxy
    .split(/[,\s]/)
    .filter(Boolean)
    .map((entry) => {
      const bracketed = /^\[(.+)\]:(\d+)$/.exec(entry);
      if (bracketed)
        return { hostname: bracketed[1], port: Number(bracketed[2]) };
      const host = entry.replace(/^\[(.+)\]$/, "$1");
      // A bare IPv6 address has several colons; only host:port has one.
      const withPort =
        (host.match(/:/g) ?? []).length === 1 && /^(.+):(\d+)$/.exec(host);
      return {
        hostname: withPort ? withPort[1] : host,
        port: withPort ? Number(withPort[2]) : 0,
      };
    })
    .map(({ hostname, port }) => ({
      hostname: hostname
        .replace(/^\*?\./, "")
        .replace(/^(.+)\.$/, "$1")
        .toLowerCase(),
      port,
    }));
}

/**
 * Whether NO_PROXY sends `url` around the proxy. The same matching as
 * undici 8.10.2's EnvHttpProxyAgent, which routes web_search: `*` alone
 * bypasses everything, an entry matches its host and every subdomain (with
 * or without a leading `.` or `*.`), and an entry's port must match.
 */
export function bypassesProxy(url, noProxy = "") {
  const entries = noProxyEntries(noProxy);
  if (entries.length === 0) return false;
  if (noProxy === "*") return true;
  let hostname = url.host
    .replace(/:\d*$/, "")
    .replace(/^\[(.+)\]$/, "$1")
    .toLowerCase();
  if (hostname.length > 1 && hostname.endsWith("."))
    hostname = hostname.slice(0, -1);
  const port =
    Number.parseInt(url.port, 10) || DEFAULT_PORTS[url.protocol] || 0;
  return entries.some(
    (entry) =>
      (!entry.port || entry.port === port) &&
      (hostname === entry.hostname || hostname.endsWith(`.${entry.hostname}`)),
  );
}

/**
 * Proxy failures before any tunnel exists, as undici 8.10.2 reports them
 * (the rule of assistant-runtime/src/runtime.mjs): the proxy refused CONNECT
 * (a bare UND_ERR_ABORTED is not one), it closed or failed TLS while
 * connecting, or SOCKS5 failed. Nothing reached the target.
 */
export function proxyRefused(error) {
  const pending = [error];
  for (let seen = 0; pending.length > 0 && seen < 20; seen++) {
    const item = pending.shift();
    if (!item || typeof item !== "object") continue;
    const code = typeof item.code === "string" ? item.code : "";
    if (
      (code === "UND_ERR_ABORTED" &&
        /^Proxy response \(\d+\) !== 200 when HTTP Tunneling$/.test(
          item.message,
        )) ||
      code === "UND_ERR_PRX_CONN" ||
      code === "UND_ERR_PRX_TLS" ||
      code.startsWith("UND_ERR_SOCKS5")
    )
      return true;
    pending.push(
      item.cause,
      ...(Array.isArray(item.errors) ? item.errors : []),
    );
  }
  return false;
}
