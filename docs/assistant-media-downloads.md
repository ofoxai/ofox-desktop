# Assistant media downloads

Only video results use gateway-provided output URLs. Images return bounded base64
and speech returns binary data from the selected gateway. The model's gateway URL
and credentials still come from the selected Ofox apex; an artifact host never
receives the gateway's Authorization header, cookies or proxy credentials.

The unsigned video URL must be HTTPS, bounded and free of URL user credentials.
Known gateway keys in raw or percent-encoded URL text are refused. Local names,
private/reserved IPv4, mapped/transition IPv6 and non-global IPv6 are rejected.
Every resolved address must pass; an empty, mixed or excessive answer fails
closed. The validated addresses are immutable for that download, so a later DNS
answer cannot change its socket destination.

Direct connections use checked literal socket addresses. With a configured or
system proxy, HTTP/HTTPS CONNECT and SOCKS5/5h also carry a literal target address,
not a hostname for a second proxy-side resolution. TLS checks the original
hostname with Mozilla and native system roots, including the user's existing
trusted proxy CAs; the HTTP Host remains that original authority. Proxy credentials
are decoded only for the proxy handshake. Explicit NO_PROXY bypasses apply by
host/port or literal CIDR. Proxy failure never silently changes to a direct route.
The proxy remains trusted user-selected infrastructure.

The sole DNS exception is an OS answer consisting entirely of the common fake-IP
range 198.18.0.0/15. As with the web-reading transport, the fixed infrastructure
endpoint `https://dns.alidns.com/resolve` supplies A/AAAA answers over HTTPS. These
answers must still all be public and are pinned before connecting. Ordinary
private or mixed answers never trigger this fallback. DNS replies are keyless,
bounded to 64 KiB, time-limited and not redirected. The resolver URL is fixed DNS
infrastructure rather than an Ofox runtime/auth/catalog/key endpoint.

Downloads have bounded connection/overall deadlines, no redirects, no automatic
generation replay and no transparent compression. Data is streamed to a private
0600 file under the owner's 0700 `.downloads` directory, at most 200 MiB, with the
existing MP4 signature check. Three process-wide staging permits remain held
through file publication or cancellation, including the final attachment copy.
The socket driver is owned and aborted when the response is dropped.

Partial filenames are generated ownership markers, never provider filenames or
paths from the renderer. They never enter conversation ZIP backups. Open file
handles are used for validation/publication; sweeping an active Unix pathname
does not invalidate its handle. Windows busy files remain until close/next sweep.
Cancellation, errors and normal shutdown drop owned files; startup and the next
download sweep identifiable orphans without following linked staging directories
or removing unrelated names/directories. Unidentifiable anonymous temp files from
older versions are left to the OS.

Tests use local TLS servers, public test-only certificates and temporary homes.
They verify the checked CONNECT/SOCKS addresses, original TLS/Host, credential
separation, proxy failure/bypass, bounds, rejected certificates, cancellation,
orphans and backup exclusion. They do not validate a real proxy installation or
native Windows behavior. Final macOS/live acceptance remains a separate gate.
