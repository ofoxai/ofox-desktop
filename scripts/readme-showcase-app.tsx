import { useEffect, useLayoutEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { queryClient } from "@/lib/query/queryClient";
import { UpdateProvider } from "@/contexts/UpdateContext";
import { ShellFrame } from "@/components/shell/ShellFrame";
import {
  ShellProvider,
  useShell,
  shellPanelId,
  type ShellPage,
} from "@/components/shell/ShellContext";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ToolsPanel } from "@/components/console/ToolsPanel";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { LibraryPage } from "@/components/library/LibraryPage";
import { ProfileDialogs } from "@/components/model-profiles/ModelProfilesPanel";
import { useModelProfiles } from "@/hooks/useModelProfiles";
import { Toaster } from "@/components/ui/sonner";
import { ThemeProvider } from "@/components/theme-provider";
import {
  ReadmeOnboarding,
  getReadmeOnboardingVersions,
} from "./readme-onboarding";
import {
  account,
  BOUND_TOOLS,
  handleToolsIpc,
  profiles,
} from "./readme-showcase-data";
import "@/index.css";

type OtherPages = typeof import("./readme-pages");
let otherPages: OtherPages | null = null;
let onboardingScene = false;
type Diagnostics = {
  ready: boolean;
  blockedIpc: string[];
  blockedNetwork: string[];
};
export function handleShowcaseIpc(
  command: string,
  payload: Record<string, unknown>,
) {
  if (command === "get_settings") return { skillSyncMethod: "copy" };
  if (onboardingScene && command === "get_tool_versions")
    return getReadmeOnboardingVersions();
  if (otherPages?.supportedReadmePageIpc.some((name) => name === command))
    return otherPages.handleReadmePageIpc(command, payload);
  return handleToolsIpc(command, payload);
}
function ProfileScene() {
  const controller = useModelProfiles(BOUND_TOOLS);
  const started = useRef(false);
  useEffect(() => {
    if (!started.current && !controller.profiles.isPending) {
      started.current = true;
      void controller.preview(profiles[1]);
    }
  }, [controller]);
  return <ProfileDialogs controller={controller} />;
}
const readmeSkillsLocation = { kind: "skills" as const, request: 1 };
function Showcase({
  page,
  scene,
  language,
  diagnostics,
}: {
  page: ShellPage;
  scene: string;
  language: string;
  diagnostics: Diagnostics;
}) {
  const shell = useShell();
  useLayoutEffect(() => {
    shell.setActive(page);
  }, [page]);
  useEffect(() => {
    if (
      shell.active !== "assistant" ||
      !["conversation", "media"].includes(scene)
    )
      return;
    // Scene initialization only: use the production row's selection/detail-read
    // path. No assistant send, settings write or native action is dispatched.
    let opened = false;
    const openSample = () => {
      const row = document.querySelector<HTMLButtonElement>(
        scene === "media"
          ? 'button[id$="-readme-media-session"]'
          : 'button[id$="-readme-session-0"]',
      );
      if (!opened && row && !row.disabled) {
        opened = true;
        observer.disconnect();
        row.click();
      }
    };
    const observer = new MutationObserver(openSample);
    observer.observe(document.getElementById("root")!, {
      childList: true,
      subtree: true,
    });
    openSample();
    return () => observer.disconnect();
  }, [shell.active, scene]);
  useEffect(() => {
    const timer = setTimeout(() => {
      diagnostics.ready = true;
    }, 500);
    return () => clearTimeout(timer);
  }, [diagnostics]);
  const OtherPage = otherPages!.ReadmeOtherPage;
  return (
    <>
      <ShellFrame account={{ kind: "signed", account }} banner={null}>
        <div
          role="tabpanel"
          id={shellPanelId(shell.active)}
          aria-label={i18n.t(`shell.tabs.${shell.active}`)}
          className={
            shell.active === "assistant"
              ? "h-full"
              : "h-full overflow-y-auto pb-4"
          }
        >
          {shell.active === "tools" ? (
            <>
              <ToolsPanel boundTools={BOUND_TOOLS} account={account} />
              {scene === "profile" && <ProfileScene />}
            </>
          ) : shell.active === "assistant" ? (
            <AssistantPage enabled />
          ) : shell.active === "library" && scene === "skills" ? (
            <LibraryPage location={readmeSkillsLocation} />
          ) : (
            <OtherPage page={shell.active} scene={scene} lang={language} />
          )}
        </div>
      </ShellFrame>
      <span className="pointer-events-none fixed bottom-2 right-3 z-[100] rounded-full border border-line bg-canvas/95 px-2.5 py-1 text-[10px] text-ink-muted">
        {{
          en: "Sample workspace · illustrative data",
          zh: "演示工作区 · 示例数据",
          ja: "デモワークスペース · サンプルデータ",
        }[language] ?? "Sample workspace"}
      </span>
      <Toaster />
    </>
  );
}
export async function mountShowcase({
  params,
  language,
  diagnostics,
}: {
  params: URLSearchParams;
  language: string;
  diagnostics: Diagnostics;
}) {
  await i18n.use(initReactI18next).init({
    lng: language,
    fallbackLng: "en",
    resources: {
      en: { translation: en },
      zh: { translation: zh },
      ja: { translation: ja },
    },
    interpolation: { escapeValue: false },
  });
  otherPages = await import("./readme-pages");
  otherPages.installReadmePageMocks();
  onboardingScene = params.get("page") === "onboarding";
  const page = (
    ["tools", "sessions", "library", "assistant", "usage"].includes(
      params.get("page") ?? "",
    )
      ? params.get("page")
      : "tools"
  ) as ShellPage;
  createRoot(document.getElementById("root")!).render(
    <ThemeProvider
      defaultTheme={params.get("theme") === "light" ? "light" : "dark"}
    >
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <UpdateProvider>
            <ShellProvider openSettings={() => undefined}>
              {onboardingScene ? (
                <ReadmeOnboarding
                  scene={params.get("scene") ?? "discovery"}
                  language={language}
                  diagnostics={diagnostics}
                  onOpenConsole={() => {
                    onboardingScene = false;
                  }}
                  renderConsole={() => (
                    <Showcase
                      page={
                        params.get("scene") === "skills" ||
                        params.get("scene") === "mcp-preview"
                          ? "library"
                          : "tools"
                      }
                      scene={
                        params.get("scene") === "skills" ||
                        params.get("scene") === "mcp-preview"
                          ? params.get("scene")!
                          : ""
                      }
                      language={language}
                      diagnostics={diagnostics}
                    />
                  )}
                />
              ) : (
                <Showcase
                  page={page}
                  scene={params.get("scene") ?? ""}
                  language={language}
                  diagnostics={diagnostics}
                />
              )}
            </ShellProvider>
          </UpdateProvider>
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>,
  );
}
