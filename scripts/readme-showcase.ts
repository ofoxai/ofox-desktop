import { installReadmeSandbox } from "./readme-showcase-guard.mjs";

let dispatch: (command: string, payload: Record<string, unknown>) => unknown = (
  command,
) => {
  throw new Error(`README sandbox has no handler: ${command}`);
};
const diagnostics = installReadmeSandbox(
  window,
  (command: string, payload: Record<string, unknown>) =>
    dispatch(command, payload),
);
const params = new URLSearchParams(location.search);
const language = ["en", "zh", "ja"].includes(params.get("lang") ?? "")
  ? params.get("lang")!
  : "en";
localStorage.setItem("language", language);
document.documentElement.lang = language;
document.documentElement.classList.toggle(
  "dark",
  params.get("theme") !== "light",
);
const application = await import("./readme-showcase-app");
dispatch = application.handleShowcaseIpc;
await application.mountShowcase({ params, language, diagnostics });
