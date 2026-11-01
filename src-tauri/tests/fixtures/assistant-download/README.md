# Public test-only TLS leaf

These are synthetic, public fixtures, never production credentials. The leaf is
for `example.com`, with CA:FALSE and serverAuth, signed by the already-public
search-MCP test CA in assistant-runtime/search-mcp/test/fixtures/tls/. The CA's
original self-signed certificate has CA:TRUE; rustls correctly refuses that CA as
an end-entity server certificate. Tests explicitly trust this CA in their own
client configuration. Production trust stores never load these files.

The committed private key exists solely to run local TLS fixture servers. Do not
use either the CA or leaf to secure real services or install them in system trust.
