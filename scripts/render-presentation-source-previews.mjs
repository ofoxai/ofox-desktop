// Offline rasterization of the actual pinned HTML, not Ofox's eight-page samples.
// Requires an existing Chrome executable. Never installs dependencies or fetches
// remote fonts/scripts. A fresh private browser profile is removed on completion.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  activationProofAccepted,
  buildCoverage,
  hashBytes,
  LEGACY_ACTIVATION,
  loadPreviewDependencies,
  RESOURCE_PATH,
} from "./presentation-resource-coverage.mjs";
import { publishSourcePreviewBatch } from "./source-preview-publish.mjs";

const root = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
const base = path.join(root, RESOURCE_PATH);
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  let serial = 0;
  const pending = new Map();
  const listeners = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const request = pending.get(message.id);
      if (!request) return;
      pending.delete(message.id);
      clearTimeout(request.timer);
      if (message.error)
        request.reject(new Error(JSON.stringify(message.error)));
      else request.resolve(message.result);
    } else
      for (const listener of listeners.get(message.method) || [])
        listener(message.params);
  });
  return {
    send(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = ++serial;
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`Chrome timeout: ${method}`));
        }, 15000);
        pending.set(id, { resolve, reject, timer });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
    on(method, listener) {
      if (!listeners.has(method)) listeners.set(method, []);
      listeners.get(method).push(listener);
    },
    close() {
      for (const request of pending.values()) {
        clearTimeout(request.timer);
        request.reject(new Error("Chrome connection closed"));
      }
      pending.clear();
      socket.close();
    },
  };
}

function activationProofExpression(selector, pageCount, index) {
  return `(() => {const nodes=Array.from(document.querySelectorAll(${JSON.stringify(selector)}));if(nodes.length!==${pageCount})throw new Error('Unexpected original page count');const node=nodes[${index}];const r=node.getBoundingClientRect();const w=Math.max(0,Math.min(r.right,innerWidth)-Math.max(r.left,0));const h=Math.max(0,Math.min(r.bottom,innerHeight)-Math.max(r.top,0));let text=0,visible=0;for(const el of node.querySelectorAll('*')){if(/^(STYLE|SCRIPT|TITLE|DESC|TEMPLATE|NOSCRIPT)$/i.test(el.tagName)||!Array.from(el.childNodes).some((n)=>n.nodeType===3&&n.textContent.trim())||!el.checkVisibility())continue;text++;const b=el.getBoundingClientRect();if(el.checkVisibility({opacityProperty:true,visibilityProperty:true})&&b.width>0&&b.height>0&&b.right>0&&b.left<innerWidth&&b.bottom>0&&b.top<innerHeight)visible++;}return{viewportCoverage:Math.round(w*h/(innerWidth*innerHeight)*1000)/1000,ownAreaVisible:r.width*r.height?Math.round(w*h/(r.width*r.height)*1000)/1000:0,rect:[r.left,r.top,r.width,r.height].map(Math.round),textElements:text,visibleTextElements:visible};})()`;
}

export async function renderOriginalHtmlPreviews(
  ids,
  chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
) {
  const coverage = await buildCoverage(root, { sourceOnly: true });
  const dependencies = await loadPreviewDependencies(root);
  const styles = coverage.styles.filter((style) => ids.includes(style.id));
  if (
    styles.length !== ids.length ||
    styles.some((style) => style.sourceDeck.kind !== "html")
  )
    throw new Error("Only known HTML source decks can be rendered");
  const catalog = JSON.parse(
    await readFile(path.join(base, "catalog.json"), "utf8"),
  );
  const allowed = new Set(
    [...catalog.sources, ...catalog.styles].flatMap((owner) =>
      (owner.resources || []).map((file) => file.path),
    ),
  );
  const server = createServer(async (request, response) => {
    try {
      const relative = decodeURIComponent(
        new URL(request.url, "http://localhost").pathname,
      ).replace(/^\//, "");
      if (!allowed.has(relative)) {
        response.writeHead(404).end();
        return;
      }
      const bytes = await readFile(path.join(base, relative));
      response.writeHead(200, {
        "Content-Type": relative.endsWith(".html")
          ? "text/html; charset=utf-8"
          : relative.endsWith(".css")
            ? "text/css"
            : relative.endsWith(".js")
              ? "text/javascript"
              : "application/octet-stream",
        "Cache-Control": "no-store",
      });
      response.end(bytes);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const evidenceRoot = path.join(
    root,
    ".real-eval/ppt-source-coverage-2026-11-10",
  );
  await mkdir(evidenceRoot, { recursive: true });
  const profile = await mkdtemp(path.join(evidenceRoot, "isolated-chrome-"));
  const browser = spawn(
    chrome,
    [
      "--headless",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-background-networking",
      "--disable-sync",
      "--disable-extensions",
      "--disable-component-update",
      "--disable-features=MediaRouter,OptimizationHints",
      "--remote-debugging-address=127.0.0.1",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  let cdp;
  try {
    let port;
    for (let attempt = 0; attempt < 100; attempt++) {
      try {
        port = (
          await readFile(path.join(profile, "DevToolsActivePort"), "utf8")
        ).split("\n")[0];
        break;
      } catch {
        await delay(100);
      }
    }
    if (!port)
      throw new Error(
        "An existing headless Chrome could not start; no browser will be installed",
      );
    const target = await (
      await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, {
        method: "PUT",
      })
    ).json();
    cdp = await connect(target.webSocketDebuggerUrl);
    const blocked = new Set();
    const runtimeErrors = new Set();
    cdp.on("Fetch.requestPaused", ({ requestId, request }) => {
      const mapped = dependencies.find(
        (dependency) => dependency.originalUrl === request.url,
      );
      if (mapped) {
        void readFile(path.join(base, mapped.path))
          .then((bytes) =>
            cdp.send("Fetch.fulfillRequest", {
              requestId,
              responseCode: 200,
              responseHeaders: [
                { name: "Content-Type", value: "text/javascript" },
              ],
              body: bytes.toString("base64"),
            }),
          )
          .catch(() =>
            cdp
              .send("Fetch.failRequest", {
                requestId,
                errorReason: "BlockedByClient",
              })
              .catch(() => {}),
          );
        return;
      }
      const local =
        request.url.startsWith(origin + "/") || request.url.startsWith("data:");
      if (!local) blocked.add(request.url);
      cdp
        .send(
          local ? "Fetch.continueRequest" : "Fetch.failRequest",
          local ? { requestId } : { requestId, errorReason: "BlockedByClient" },
        )
        .catch(() => {});
    });
    cdp.on("Runtime.exceptionThrown", ({ exceptionDetails }) =>
      runtimeErrors.add(
        exceptionDetails.exception?.description || exceptionDetails.text,
      ),
    );
    await cdp.send("Fetch.enable", {
      patterns: [{ urlPattern: "*", requestStage: "Request" }],
    });
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1920,
      height: 1080,
      deviceScaleFactor: 1,
      mobile: false,
    });
    const version = await cdp.send("Browser.getVersion");
    const records = [];
    const outputFiles = new Map();
    for (const style of styles) {
      blocked.clear();
      runtimeErrors.clear();
      const sourceFile = style.sourceDeck.files.find((file) =>
        file.path.endsWith(".html"),
      );
      await cdp.send("Page.navigate", { url: `${origin}/${sourceFile.path}` });
      await delay(1000);
      // The five first-reviewed decks keep their evidenced forced-display
      // capture. Every other deck is driven by its own keyboard navigation,
      // so entrance animations reach their authored end state, and each page
      // carries a DOM proof that it filled the viewport with visible text.
      const native = !LEGACY_ACTIVATION.has(style.id);
      if (native) {
        const prepared = await cdp.send("Runtime.evaluate", {
          expression: `(() => {const css=document.createElement('style');css.textContent='*,*::before,*::after{animation-duration:1ms!important;animation-delay:0ms!important;transition-duration:1ms!important;transition-delay:0ms!important}.nav-controls,.slide-counter,.keyboard-hint,.nav-bar,.progress-bar,.overview,.notes-overlay{display:none!important}';document.head.appendChild(css);return true;})()`,
          returnByValue: true,
        });
        if (prepared.exceptionDetails)
          throw new Error(`Original deck preparation failed: ${style.id}`);
        await delay(900);
      }
      const pages = [];
      const omittedPages = [];
      const output = path.join(base, "source-previews", style.id);
      await mkdir(output, { recursive: true });
      for (const page of style.sourceDeck.pages) {
        let activationProof;
        const history = [];
        if (native) {
          for (let presses = 0; ; presses++) {
            const proof = await cdp.send("Runtime.evaluate", {
              expression: activationProofExpression(
                page.selector,
                style.sourceDeck.pageCount,
                page.index,
              ),
              returnByValue: true,
            });
            if (proof.exceptionDetails)
              throw new Error(
                `Original page proof failed: ${style.id}/${page.number}`,
              );
            activationProof = { ...proof.result.value, keyPresses: presses };
            history.push(activationProof);
            if (activationProofAccepted(activationProof)) break;
            if (presses === 6)
              throw new Error(
                `Original page never became visible through the deck's own navigation: ${style.id}/${page.number} ${JSON.stringify(history)}`,
              );
            for (const type of ["keyDown", "keyUp"])
              await cdp.send("Input.dispatchKeyEvent", {
                type,
                key: "ArrowRight",
                code: "ArrowRight",
                windowsVirtualKeyCode: 39,
                nativeVirtualKeyCode: 39,
              });
            await delay(900);
          }
        }
        const expression = native
          ? `({pages:${style.sourceDeck.pageCount},selected:${page.number}})`
          : `(() => { const nodes = Array.from(document.querySelectorAll(${JSON.stringify(page.selector)})); if(nodes.length!==${style.sourceDeck.pageCount}) throw new Error('Unexpected original page count'); const css=document.createElement('style');css.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';document.head.appendChild(css); nodes.forEach((node,index)=>{const active=index===${page.index};node.style.display=active?'':'none';node.classList.toggle('active',active);node.classList.toggle('is-active',active);if(active){node.style.opacity='1';node.style.transform='none';node.style.pointerEvents='auto';}});for(const chrome of document.querySelectorAll('.nav-controls,.slide-counter,.keyboard-hint,.nav-bar,.progress-bar,.overview,.notes-overlay'))chrome.style.display='none';return {pages:nodes.length,selected:${page.number},width:innerWidth,height:innerHeight,title:document.title};})()`;
        const result = await cdp.send("Runtime.evaluate", {
          expression,
          returnByValue: true,
        });
        if (
          result.exceptionDetails ||
          result.result.value?.selected !== page.number
        )
          throw new Error(
            `Original page activation failed: ${style.id}/${page.number}`,
          );
        await delay(80);
        let chartEvidence;
        if (style.id === "cartesian" && [4, 7].includes(page.number)) {
          const proof = await cdp.send("Runtime.evaluate", {
            expression: `(() => {const canvas=document.querySelector(${JSON.stringify(page.selector)}+':nth-child(${page.number}) canvas');const chart=globalThis.Chart?.getChart(canvas);if(!chart)throw Error('Original Chart.js canvas not initialized');chart.stop();chart.resize();chart.update('none');const rgba=canvas.getContext('2d').getImageData(0,0,canvas.width,canvas.height).data;let ink=0;for(let at=3;at<rgba.length;at+=4)if(rgba[at])ink++;return{libraryVersion:Chart.version,datasets:chart.data.datasets.map(({label,data})=>({label,data})),nonTransparentPixels:ink,canvasWidth:canvas.width,canvasHeight:canvas.height};})()`,
            returnByValue: true,
          });
          if (
            proof.exceptionDetails ||
            !(proof.result.value?.nonTransparentPixels > 1000)
          )
            throw new Error(
              `Original chart remains blank: ${style.id}/${page.number}`,
            );
          chartEvidence = proof.result.value;
        }
        const image = await cdp.send("Page.captureScreenshot", {
          format: "jpeg",
          quality: 88,
          captureBeyondViewport: false,
          clip: { x: 0, y: 0, width: 1920, height: 1080, scale: 1 },
        });
        const bytes = Buffer.from(image.data, "base64");
        const relative = `source-previews/${style.id}/page-${String(page.number).padStart(2, "0")}.jpg`;
        outputFiles.set(relative, bytes);
        pages.push({
          number: page.number,
          path: relative,
          sha256: hashBytes(bytes),
          width: 1920,
          height: 1080,
          ...(chartEvidence ? { chartEvidence } : {}),
          ...(activationProof ? { activationProof } : {}),
        });
      }
      records.push({
        id: style.id,
        status: omittedPages.length
          ? "partially-rendered"
          : "rendered-with-local-fonts",
        pageCount: style.sourceDeck.pageCount,
        sourceSha256: sourceFile.sha256,
        inputFiles: [
          ...style.sourceDeck.files,
          ...style.sourceDeck.dependencyFiles,
        ].map(({ path: filePath, sha256 }) => ({ path: filePath, sha256 })),
        dependencyOverrides: style.id === "cartesian" ? dependencies : [],
        renderer: {
          name: version.product,
          executable: chrome,
          viewport: { width: 1920, height: 1080 },
          freshProfile: true,
          network:
            "Task-owned loopback source server and data URLs only. The exact original Chart.js URL is fulfilled from the reviewed pinned preview-only local bytes; all other remote page requests blocked.",
          activationMode: native ? "native-navigation" : "forced-display",
          activation: native
            ? "The deck's own ArrowRight navigation; animations and transitions shortened to their authored end state and navigation chrome hidden. Original source bytes remain unchanged."
            : "Original selected page only; animation/transition paused and navigation chrome hidden. Original source bytes remain unchanged.",
        },
        fontPolicy:
          "Google Fonts blocked; existing local system fallbacks are used. This is not original-font or cross-platform fidelity acceptance.",
        pages,
        omittedPages,
        blockedUrls: [...blocked].sort(),
        runtimeErrors: [...runtimeErrors].sort(),
        visualReview: "pending",
        nativeEditing: "not-inferred",
      });
      console.log(
        JSON.stringify({
          style: style.id,
          originalPages: style.sourceDeck.pageCount,
          rendered: pages.length,
          omitted: omittedPages.length,
        }),
      );
    }
    return await publishSourcePreviewBatch(base, records, outputFiles, {
      stagingRoot: evidenceRoot,
    });
  } finally {
    cdp?.close();
    browser.kill("SIGTERM");
    await new Promise((resolve) => {
      if (browser.exitCode !== null) resolve();
      else {
        browser.once("exit", resolve);
        setTimeout(resolve, 3000).unref();
      }
    });
    await new Promise((resolve) => server.close(resolve));
    await rm(profile, { recursive: true, force: true });
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const requested = process.argv
    .find((arg) => arg.startsWith("--styles="))
    ?.slice(9)
    .split(",") || [
    "blue-professional",
    "cartesian",
    "course-module",
    "product-launch",
  ];
  await renderOriginalHtmlPreviews(requested);
}
