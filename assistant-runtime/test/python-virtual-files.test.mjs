import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { loadPyodide } from "pyodide";
import { writeDeckModule } from "../src/python/virtual-files.mjs";

test("the real Pyodide module stays importable with every host path API", async (t) => {
  // Fixed invented module; writes stay in MEMFS and no wheels are loaded.
  const py = await loadPyodide();
  const site = py.runPython("import sysconfig; sysconfig.get_path('purelib')");
  const source = "VALUE = 42\n";
  const legacy = path.win32.join(site, "deck.py");
  py.FS.writeFile(legacy, source);
  assert.equal(py.FS.readFile(legacy, { encoding: "utf8" }), source);
  assert.throws(
    () => py.runPython("import deck"),
    /ModuleNotFoundError/,
    "Windows separators write a literal filename outside Python's module search path",
  );
  py.FS.unlink(legacy);

  for (const [host, platformPath] of [
    ["Windows", path.win32],
    ["POSIX", path.posix],
    ["current host", undefined],
  ]) {
    await t.test(host, () => {
      try {
        writeDeckModule(py.FS, site, Buffer.from(source), platformPath);
        assert.equal(py.runPython("import deck; deck.VALUE"), 42);
      } finally {
        py.runPython("import sys; sys.modules.pop('deck', None)");
        for (const name of [path.posix.join(site, "deck.py"), legacy])
          if (py.FS.analyzePath(name).exists) py.FS.unlink(name);
      }
    });
  }
});
