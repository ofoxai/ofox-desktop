/** UI-only sandbox. No IPC, network, source configuration, or paid calls. */
import { createRoot } from "react-dom/client";
import { useState } from "react";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
} from "@/lib/api/library";
import { skillsApi } from "@/lib/api/skills";
import { LibraryScenarioWizard } from "@/components/library/LibraryScenarioWizard";
import { LibraryIssueNotice } from "@/components/library/LibraryIssueNotice";
import { checkIssue } from "@/components/library/libraryIssues";
import { Button } from "@/components/ui/button";
import "@/index.css";

void i18n
  .use(initReactI18next)
  .init({
    lng: "zh",
    resources: {
      zh: { translation: zh },
      en: { translation: en },
      ja: { translation: ja },
    },
    interpolation: { escapeValue: false },
  });
const sets: NonNullable<LibraryData["instructions"]> = [];
const servers: NonNullable<LibraryData["servers"]> = [];
const skills: NonNullable<LibraryData["skills"]> = [];
const targets = ["claude", "codex", "gemini", "opencode", "hermes"].map(
  (toolId) => ({
    toolId,
    available: true,
    discoveryStatus: "installed",
    path: `/sandbox/${toolId}`,
    targetGroupId: toolId,
    status: "unassigned",
  }),
);
libraryApi.list = async () => ({
  epoch: 1,
  instructions: sets,
  servers,
  skills,
  targets,
  operations: [],
});
libraryApi.matchBuiltinRecipes = async () => [];
libraryApi.matchMcpRecipes = async () => [];
libraryApi.saveInstruction = async (value) => {
  const item = { ...value, revision: 1 };
  sets.push(item);
  return item;
};
libraryApi.saveMcp = async (id, name) => {
  servers.push({
    id,
    name,
    revision: 1,
    transport: "http",
    credentialState: "none",
    fields: [],
    targets: [],
  });
};
skillsApi.installUnified = async () => {
  throw new Error("fixture: downloads are blocked");
};
libraryApi.previewBatch = async (kind, resources) => ({
  operationId: `fixture-${kind}`,
  kind,
  status: "previewed",
  resources,
  groups: [
    {
      groupId: "fixture",
      tools: ["codex"],
      readers: ["codex"],
      path: "/sandbox/config",
      status: "pending",
      resourceIds: resources.map((item) => item.resourceId),
    },
  ],
});
libraryApi.cancelBatch = async () => ({
  operationId: "fixture",
  kind: "mcp",
  status: "cancelled",
  resources: [],
  groups: [],
});
libraryApi.runBatch = async () => {
  throw new Error("fixture: native writes are blocked");
};
const issue = checkIssue({
  operationId: "fixture",
  state: "blocked",
  reason: "runnerBlocked",
  tools: [],
})!;
function Fixture() {
  const [language, setLanguage] = useState("zh");
  return (
    <main className="mx-auto max-w-[1200px] space-y-5 p-4">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <b className="text-title">OFox · {i18n.t("library.nav")}</b>
        <div className="flex gap-2">
          {["zh", "en", "ja"].map((value) => (
            <Button
              key={value}
              variant={value === language ? "primary" : "secondary"}
              onClick={() => {
                setLanguage(value);
                void i18n.changeLanguage(value);
              }}
            >
              {value}
            </Button>
          ))}
          <Button
            variant="secondary"
            onClick={() => document.documentElement.classList.toggle("dark")}
          >
            Theme
          </Button>
        </div>
      </header>
      <section className="space-y-3 rounded-card border border-line bg-card p-4">
        <p className="text-note text-ink-muted">
          UI fixture · synthetic resources · all native writes and downloads
          blocked
        </p>
        <LibraryScenarioWizard onChanged={async () => undefined} />
      </section>
      <LibraryIssueNotice issue={issue} />
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<Fixture />);
