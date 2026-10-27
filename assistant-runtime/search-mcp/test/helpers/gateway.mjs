import http from "node:http";

// A local stand-in for the Ofox gateway on an ephemeral port. Every request is
// recorded (method, path, headers, raw body) before `handler` answers it.
export async function startGateway(handler) {
  const requests = [];
  const server = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const record = {
      method: request.method,
      url: request.url,
      headers: request.headers,
      body: Buffer.concat(chunks).toString("utf8"),
    };
    requests.push(record);
    await handler(request, response, record);
  });
  await new Promise((resolve) => server.listen(0, "localhost", resolve));
  return {
    baseUrl: `http://localhost:${server.address().port}`,
    requests,
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}

export function replyJson(response, status, value) {
  const body = Buffer.from(JSON.stringify(value));
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": body.byteLength,
  });
  response.end(body);
}
