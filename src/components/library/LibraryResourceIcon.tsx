import { FileText, Plug, Puzzle } from "lucide-react";
import microsoftLogo from "@lobehub/icons-static-svg/icons/microsoft-color.svg";
import githubLogo from "@lobehub/icons-static-svg/icons/github.svg";
import anthropicLogo from "@lobehub/icons-static-svg/icons/anthropic.svg";
import mcpLogo from "@lobehub/icons-static-svg/icons/mcp.svg";
import type { LibraryKind } from "@/lib/api/library";
import { cn } from "@/lib/utils";

/** Only pass a maintained recipe ID or a backend-confirmed recipe match. */
const RECIPE_LOGOS: Readonly<
  Record<string, { src: string; invert?: boolean }>
> = {
  "microsoft-learn": { src: microsoftLogo },
  playwright: { src: microsoftLogo },
  github: { src: githubLogo, invert: true },
  filesystem: { src: mcpLogo, invert: true },
  fetch: { src: mcpLogo, invert: true },
  "frontend-design": { src: anthropicLogo, invert: true },
  "webapp-testing": { src: anthropicLogo, invert: true },
  "mcp-builder": { src: anthropicLogo, invert: true },
  "skill-creator": { src: anthropicLogo, invert: true },
  "theme-factory": { src: anthropicLogo, invert: true },
  "canvas-design": { src: anthropicLogo, invert: true },
};

export function LibraryResourceIcon({
  recipeId,
  kind,
  className,
}: {
  recipeId?: string;
  kind: LibraryKind;
  className?: string;
}) {
  const logo =
    recipeId && Object.prototype.hasOwnProperty.call(RECIPE_LOGOS, recipeId)
      ? RECIPE_LOGOS[recipeId]
      : undefined;
  const Icon = kind === "mcp" ? Plug : kind === "skills" ? Puzzle : FileText;
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex h-10 w-10 shrink-0 items-center justify-center rounded-ctl bg-pill text-ink-2",
        className,
      )}
    >
      {logo ? (
        <img
          src={logo.src}
          alt=""
          className={cn("h-6 w-6 object-contain", logo.invert && "dark:invert")}
        />
      ) : (
        <Icon className="h-5 w-5" />
      )}
    </span>
  );
}
