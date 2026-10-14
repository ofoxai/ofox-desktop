import fs from "node:fs";
import path from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";
const root = path.resolve(__dirname, "../..");
const scopes = [
  "console",
  "shell",
  "sessions",
  "usage-overview",
  "tray",
  "onboarding",
  "model-profiles",
  "ui",
];
const languageNames = new Set(["简体中文", "中文", "日本語"]);
function files(dir: string): string[] {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((entry) =>
      entry.isDirectory()
        ? files(path.join(dir, entry.name))
        : entry.name.endsWith(".tsx")
          ? [path.join(dir, entry.name)]
          : [],
    );
}
it("keeps UI copy in locales, with only language names allowed inline", () => {
  const offenders: string[] = [];
  for (const file of scopes.flatMap((scope) =>
    files(path.join(root, "src/components", scope)),
  )) {
    const source = ts.createSourceFile(
      file,
      fs.readFileSync(file, "utf8"),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    function visit(node: ts.Node) {
      if (
        ts.isStringLiteralLike(node) ||
        ts.isTemplateHead(node) ||
        ts.isTemplateMiddle(node) ||
        ts.isTemplateTail(node) ||
        ts.isJsxText(node)
      ) {
        if (
          /[\u3400-\u9fff\u3040-\u30ff]/u.test(node.text) &&
          !languageNames.has(node.text.trim())
        ) {
          offenders.push(
            `${path.relative(root, file)}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}: ${node.text.trim()}`,
          );
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
  expect(offenders).toEqual([]);
});
