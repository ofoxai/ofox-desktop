import { test } from "node:test";
import assert from "node:assert/strict";
import { installReadmeSandbox } from "./readme-showcase-guard.mjs";

function sandbox() {
  const listeners = {};
  const target = {
    navigator: {},
    document: {
      addEventListener: (kind, fn) => {
        listeners[kind] = fn;
      },
    },
  };
  const state = installReadmeSandbox(target, async (command) => {
    if (command === "showcase_read") return { synthetic: true };
    throw new Error("unknown showcase IPC");
  });
  return { target, state, listeners };
}

test("only synthetic IPC dispatch is reachable; unknown native commands reject", async () => {
  const { target, state } = sandbox();
  assert.deepEqual(await target.__TAURI_INTERNALS__.invoke("showcase_read"), {
    synthetic: true,
  });
  await assert.rejects(
    target.__TAURI_INTERNALS__.invoke("write_real_config"),
    /unknown/,
  );
  assert.deepEqual(state.blockedIpc, ["write_real_config"]);
});
test("network transports and external window navigation are blocked", async () => {
  const { target, state } = sandbox();
  await assert.rejects(target.fetch("https://example.com"), /blocked fetch/);
  for (const kind of [
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "Worker",
    "RTCPeerConnection",
  ])
    assert.throws(() => new target[kind]("https://example.com"), /blocked/);
  assert.equal(target.navigator.sendBeacon("https://example.com"), false);
  assert.equal(target.open("https://example.com"), null);
  assert.equal(state.blockedNetwork.length, 8);
});
test("persistent browser storage is never read and sample data is page-local", () => {
  const { target } = sandbox();
  target.localStorage.setItem("language", "en");
  assert.equal(target.localStorage.getItem("language"), "en");
  const fresh = sandbox();
  assert.equal(fresh.target.localStorage.getItem("language"), null);
  assert.equal(fresh.target.sessionStorage.length, 0);
});
test("anchor navigation, forms and native asset URLs are blocked", () => {
  const { target, listeners, state } = sandbox();
  let prevented = 0;
  listeners.click({
    target: { closest: () => ({ href: "https://example.com" }) },
    preventDefault: () => prevented++,
  });
  listeners.submit({ preventDefault: () => prevented++ });
  assert.equal(prevented, 2);
  assert.equal(
    target.__TAURI_INTERNALS__.convertFileSrc("/private/file"),
    "data:,",
  );
  assert.deepEqual(state.blockedNetwork, [
    "anchor navigation",
    "form submission",
    "native asset",
  ]);
});
test("media devices and clipboard cannot expose local user data", async () => {
  const { target, state } = sandbox();
  await assert.rejects(
    target.navigator.mediaDevices.getUserMedia({ audio: true }),
    /blocked/,
  );
  await assert.rejects(
    target.navigator.mediaDevices.getDisplayMedia({ video: true }),
    /blocked/,
  );
  await assert.rejects(target.navigator.clipboard.readText(), /blocked/);
  assert.deepEqual(await target.navigator.mediaDevices.enumerateDevices(), []);
  assert.deepEqual(state.blockedNetwork, [
    "media capture",
    "screen capture",
    "clipboard read",
  ]);
});
test("paste and drop are stopped before application handlers read real files", () => {
  const { listeners } = sandbox();
  let prevented = 0;
  let stopped = 0;
  for (const name of ["paste", "drop"])
    listeners[name]({
      preventDefault: () => prevented++,
      stopImmediatePropagation: () => stopped++,
    });
  assert.equal(prevented, 2);
  assert.equal(stopped, 2);
});
