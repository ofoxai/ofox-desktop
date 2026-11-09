/** Browser-only README sandbox. Install before importing any application code. */
export function installReadmeSandbox(target, dispatch) {
  const diagnostics = {
    blockedIpc: [],
    blockedNetwork: [],
    calls: [],
    ready: false,
  };
  target.__README_SHOWCASE__ = diagnostics;
  const blocked = (kind) => {
    diagnostics.blockedNetwork.push(kind);
    throw new Error(`README sandbox blocked ${kind}`);
  };
  target.fetch = async () => blocked("fetch");
  for (const kind of [
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "Worker",
    "RTCPeerConnection",
  ])
    target[kind] = class {
      constructor() {
        blocked(kind);
      }
    };
  target.open = () => {
    diagnostics.blockedNetwork.push("window.open");
    return null;
  };
  if (target.navigator) {
    Object.defineProperty(target.navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: async () => blocked("media capture"),
        getDisplayMedia: async () => blocked("screen capture"),
        enumerateDevices: async () => [],
      },
    });
    Object.defineProperty(target.navigator, "clipboard", {
      configurable: true,
      value: {
        read: async () => blocked("clipboard read"),
        readText: async () => blocked("clipboard read"),
        write: async () => undefined,
        writeText: async () => undefined,
      },
    });
    Object.defineProperty(target.navigator, "sendBeacon", {
      configurable: true,
      value: () => {
        diagnostics.blockedNetwork.push("sendBeacon");
        return false;
      },
    });
  }
  // Each page load starts from synthetic storage; never inspect origin storage.
  for (const name of ["localStorage", "sessionStorage"]) {
    const values = new Map();
    Object.defineProperty(target, name, {
      configurable: true,
      value: {
        get length() {
          return values.size;
        },
        key: (index) => [...values.keys()][index] ?? null,
        getItem: (key) => values.get(String(key)) ?? null,
        setItem: (key, value) => values.set(String(key), String(value)),
        removeItem: (key) => values.delete(String(key)),
        clear: () => values.clear(),
      },
    });
  }
  let callbackId = 0;
  target.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "readme-showcase" },
      currentWebview: { label: "readme-showcase" },
    },
    transformCallback: () => ++callbackId,
    unregisterCallback: () => undefined,
    invoke: async (command, payload = {}) => {
      diagnostics.calls.push(command);
      if (command === "plugin:event|listen") return ++callbackId;
      if (command === "plugin:event|unlisten") return;
      try {
        return await dispatch(command, payload);
      } catch (error) {
        diagnostics.blockedIpc.push(command);
        throw error;
      }
    },
    convertFileSrc: () => {
      diagnostics.blockedNetwork.push("native asset");
      return "data:,";
    },
  };
  target.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: () => undefined,
  };
  target.document?.addEventListener(
    "click",
    (event) => {
      const anchor = event.target?.closest?.("a[href]");
      if (anchor) {
        event.preventDefault();
        diagnostics.blockedNetwork.push("anchor navigation");
      }
    },
    true,
  );
  target.document?.addEventListener(
    "submit",
    (event) => {
      event.preventDefault();
      diagnostics.blockedNetwork.push("form submission");
    },
    true,
  );
  target.document?.addEventListener("securitypolicyviolation", (event) => {
    diagnostics.blockedNetwork.push(`CSP:${event.violatedDirective}`);
  });
  for (const eventType of ["paste", "drop"])
    target.document?.addEventListener(
      eventType,
      (event) => {
        event.preventDefault();
        event.stopImmediatePropagation();
      },
      true,
    );
  return diagnostics;
}
