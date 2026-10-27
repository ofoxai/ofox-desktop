import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const bundledNode = fileURLToPath(
  new URL(
    "../../resources/node/" +
      (process.platform === "win32" ? "node.exe" : "node"),
    import.meta.url,
  ),
);
const main = fileURLToPath(new URL("../../src/main.mjs", import.meta.url));
export const inheritedContextMarker =
  "UNRELATED_PARENT_CONTEXT_MUST_NOT_REACH_MODEL";

export function writeModelStream(response, text, finish = true) {
  response.writeHead(200, { "content-type": "text/event-stream" });
  response.write(
    "data: " +
      JSON.stringify({
        id: "stdio-fixture",
        choices: [
          {
            index: 0,
            delta: { role: "assistant", content: text },
            finish_reason: null,
          },
        ],
      }) +
      "\n\n",
  );
  if (finish) {
    response.end(
      "data: " +
        JSON.stringify({
          choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
          usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        }) +
        "\n\ndata: [DONE]\n\n",
    );
  }
}

export function writeModelToolCall(response, name, args) {
  response.writeHead(200, { "content-type": "text/event-stream" });
  response.end(
    "data: " +
      JSON.stringify({
        id: "stdio-tool",
        choices: [
          {
            index: 0,
            delta: {
              role: "assistant",
              tool_calls: [
                {
                  index: 0,
                  id: "stdio-native-call",
                  type: "function",
                  function: { name, arguments: JSON.stringify(args) },
                },
              ],
            },
            finish_reason: null,
          },
        ],
      }) +
      "\n\ndata: " +
      JSON.stringify({
        choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      }) +
      "\n\ndata: [DONE]\n\n",
  );
}

function bounded(promise, milliseconds, message) {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    }),
  ]).finally(() => clearTimeout(timer));
}

export function attachPrivateStdio(child) {
  const messages = [];
  const waiters = new Set();
  let stdout = "";
  let stderr = "";
  let sequence = 0;
  let exited = false;
  let forcedTermination = false;
  child.stderr.on("data", (data) => {
    stderr += data;
  });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    stdout += line + "\n";
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      for (const waiter of waiters)
        waiter.reject(
          new Error("Non-JSON data corrupted the private stdout pipe"),
        );
      waiters.clear();
      return;
    }
    messages.push(message);
    for (const waiter of waiters) {
      if (waiter.predicate(message)) {
        waiters.delete(waiter);
        waiter.resolve(message);
      }
    }
  });
  // A process can exit while its pipes still contain the final response or
  // diagnostics. ChildProcess.close establishes that all stdio has closed.
  const exit = once(child, "close").then(([code, signal]) => {
    exited = true;
    lines.close();
    for (const waiter of waiters)
      waiter.reject(new Error("Private sidecar exited before its response"));
    waiters.clear();
    return { code, signal };
  });
  const waitFor = (predicate) => {
    const existing = messages.find(predicate);
    if (existing) return Promise.resolve(existing);
    if (exited) return Promise.reject(new Error("Private sidecar has exited"));
    let waiter;
    return bounded(
      new Promise((resolve, reject) => {
        waiter = { predicate, resolve, reject };
        waiters.add(waiter);
      }),
      15000,
      "Timed out waiting for private sidecar response",
    ).finally(() => waiters.delete(waiter));
  };
  const write = (value) => child.stdin.write(JSON.stringify(value) + "\n");
  const start = (method, params = {}) => {
    const id = ++sequence;
    const result = waitFor(
      (message) => message.id === id && !message.method,
    ).then((message) => {
      if (message.error) throw new Error(message.error.message);
      return message.result;
    });
    write({ id, method, params });
    return result;
  };
  const api = {
    messages,
    waitFor,
    start,
    write,
    writeRaw: (line) => child.stdin.write(line + "\n"),
    get stdout() {
      return stdout;
    },
    get stderr() {
      return stderr;
    },
    get forcedTermination() {
      return forcedTermination;
    },
    close: async () => {
      if (exited) return;
      child.stdin.end();
      try {
        await bounded(exit, 3000, "Sidecar did not exit after EOF");
      } catch {
        forcedTermination = true;
        child.kill();
        await bounded(exit, 3000, "Sidecar did not terminate");
      }
    },
    exit,
  };
  return api;
}

export async function createStdioAcceptance(t, handleModel) {
  const root = await mkdtemp(path.join(tmpdir(), "ofox private stdio "));
  const emptyPath = path.join(root, "empty path");
  await mkdir(emptyPath);
  await writeFile(path.join(root, "AGENTS.md"), inheritedContextMarker);
  await mkdir(path.join(root, ".pi", "agent"), { recursive: true });
  await writeFile(
    path.join(root, ".pi", "agent", "settings.json"),
    "deliberately invalid unrelated settings",
  );
  const children = [];
  const requests = [];
  const serverErrors = [];
  let server;
  t.after(async () => {
    await Promise.all(children.map((child) => child.close()));
    server?.closeAllConnections();
    if (server?.listening)
      await new Promise((resolve) => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  });
  if (handleModel) {
    server = createServer(async (request, response) => {
      try {
        assert.equal(request.method, "POST");
        assert.equal(request.url, "/v1/chat/completions");
        let bytes = "";
        for await (const chunk of request) bytes += chunk;
        const entry = {
          body: JSON.parse(bytes),
          authorization: request.headers.authorization,
        };
        requests.push(entry);
        await handleModel(entry, response, requests.length);
      } catch (error) {
        serverErrors.push(error);
        response.writeHead(500).end("Synthetic model fixture failed");
      }
    });
    server.listen(8088, "localhost");
    await once(server, "listening");
  }

  const spawnSidecar = (pathMode = "empty") => {
    const environment = {
      PATH: pathMode === "empty" ? "" : emptyPath,
      HOME: root,
      USERPROFILE: root,
      TMPDIR: root,
      TEMP: root,
      TMP: root,
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    };
    const child = spawn(bundledNode, [main], {
      cwd: root,
      env: environment,
      stdio: ["pipe", "pipe", "pipe"],
    });
    const api = attachPrivateStdio(child);
    children.push(api);
    return api;
  };
  const params = (name, token, extra = {}) => ({
    sessionId: name + "-session",
    runId: name + "-run",
    operationId: name + "-operation",
    sessionFile: path.join(root, "native session files", name + ".jsonl"),
    kind: "general",
    prompt: "Synthetic question",
    accessToken: token,
    apex: "ofox.ai",
    baseUrl: "http://localhost:8088",
    allowLocalGateway: true,
    model: {
      id: "test/stdio",
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    attachmentRoot: root,
    attachments: [],
    selectedAttachmentIds: [],
    ...extra,
  });
  const assertClean = async (tokens) => {
    await Promise.all(children.map((child) => child.close()));
    assert.equal(serverErrors.length, 0, "Synthetic model handler failed");
    for (const request of requests)
      for (const token of tokens)
        assert.equal(
          JSON.stringify(request.body).includes(token),
          false,
          "Credential escaped into the model payload",
        );
    const visit = async (directory) => {
      for (const entry of await readdir(directory, { withFileTypes: true })) {
        const filename = path.join(directory, entry.name);
        if (entry.isDirectory()) await visit(filename);
        else {
          assert.ok(entry.isFile(), "Unexpected non-regular fixture file");
          const bytes = await readFile(filename);
          for (const token of tokens)
            assert.equal(
              bytes.includes(Buffer.from(token)),
              false,
              "Credential persisted in a private fixture file",
            );
        }
      }
    };
    await visit(root);
    for (const wire of children) {
      for (const token of tokens) {
        assert.equal(
          wire.stdout.includes(token),
          false,
          "Credential escaped through stdout",
        );
        assert.equal(
          wire.stderr.includes(token),
          false,
          "Credential escaped through diagnostic stderr",
        );
      }
    }
  };
  return { root, requests, spawnSidecar, params, assertClean };
}
