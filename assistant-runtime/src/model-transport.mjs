// Model transports are owned by one turn/title request, never the global
// dispatcher. Keep the proxy's credentials confined to its own dispatcher.
const CONNECT_TIMEOUT_MS = 30000;

export async function modelTransport(proxy) {
  const { Agent, Dispatcher1Wrapper, EnvHttpProxyAgent, fetch } =
    await import("undici");
  if (proxy === undefined || proxy === null) {
    // The SDK's 15-minute request timeout does not extend fetch's separate
    // 10-second TLS connection timeout. Retain the built-in fetch's HTTP/1.1
    // policy while giving a connection the same 30 seconds as the Rust client.
    // Node 22's built-in fetch uses the legacy dispatcher handler interface.
    const dispatcher = new Dispatcher1Wrapper(
      new Agent({ connectTimeout: CONNECT_TIMEOUT_MS, allowH2: false }),
    );
    return transport(dispatcher, globalThis.fetch);
  }
  let url;
  try {
    url = new URL(proxy.url);
  } catch {
    throw new Error("The proxy setting is not a valid URL");
  }
  if (url.protocol === "socks5h:") url.protocol = "socks5:";
  if (!["http:", "https:", "socks5:"].includes(url.protocol))
    throw new Error("The proxy setting uses an unsupported scheme");
  const dispatcher = new EnvHttpProxyAgent({
    httpProxy: url.href,
    httpsProxy: url.href,
    noProxy: typeof proxy.noProxy === "string" ? proxy.noProxy : "",
    proxyTunnel: true,
    connectTimeout: CONNECT_TIMEOUT_MS,
  });
  return transport(dispatcher, fetch);
}

function transport(dispatcher, fetch) {
  return {
    fetch: (input, init) => fetch(input, { ...init, dispatcher }),
    close: () => dispatcher.destroy().catch(() => undefined),
  };
}
