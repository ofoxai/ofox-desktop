import {
  Code2,
  FileText,
  Languages,
  Plug,
  Puzzle,
  ScanSearch,
  type LucideIcon,
} from "lucide-react";
import microsoftLogo from "@lobehub/icons-static-svg/icons/microsoft-color.svg";
import githubLogo from "@lobehub/icons-static-svg/icons/github.svg";
import anthropicLogo from "@lobehub/icons-static-svg/icons/anthropic.svg";
import mcpLogo from "@lobehub/icons-static-svg/icons/mcp.svg";
import context7Logo from "@/assets/library-logos/context7.png";
import chromeLogo from "@/assets/library-logos/chrome-devtools.png";
import playwrightLogo from "@/assets/library-logos/playwright.svg";
import deepwikiLogo from "@/assets/library-logos/deepwiki.png";
import exaLogo from "@lobehub/icons-static-svg/icons/exa-color.svg";
import notionLogo from "@lobehub/icons-static-svg/icons/notion.svg";
import vercelLogo from "@lobehub/icons-static-svg/icons/vercel.svg";
import cloudflareLogo from "@lobehub/icons-static-svg/icons/cloudflare-color.svg";
import huggingfaceLogo from "@lobehub/icons-static-svg/icons/huggingface-color.svg";
import tavilyLogo from "@lobehub/icons-static-svg/icons/tavily-color.svg";
import sentryLogo from "@/assets/library-logos/sentry.png";
import linearLogo from "@/assets/library-logos/linear.png";
import atlassianLogo from "@/assets/library-logos/atlassian.png";
import supabaseLogo from "@/assets/library-logos/supabase.png";
import stripeLogo from "@/assets/library-logos/stripe.png";
import neonLogo from "@/assets/library-logos/neon.png";
import bravesearchLogo from "@/assets/library-logos/brave-search.png";
import firecrawlLogo from "@/assets/library-logos/firecrawl.png";
import type { LibraryKind } from "@/lib/api/library";
import { cn } from "@/lib/utils";

/** Only pass a maintained recipe ID or a backend-confirmed recipe match. */
const RECIPE_LOGOS: Readonly<
  Record<string, { src: string; invert?: boolean }>
> = {
  exa: { src: exaLogo },
  memory: { src: mcpLogo, invert: true },
  "sequential-thinking": { src: mcpLogo, invert: true },
  git: { src: mcpLogo, invert: true },
  time: { src: mcpLogo, invert: true },
  sentry: { src: sentryLogo },
  linear: { src: linearLogo },
  notion: { src: notionLogo, invert: true },
  atlassian: { src: atlassianLogo },
  supabase: { src: supabaseLogo },
  stripe: { src: stripeLogo },
  vercel: { src: vercelLogo, invert: true },
  neon: { src: neonLogo },
  "cloudflare-docs": { src: cloudflareLogo },
  huggingface: { src: huggingfaceLogo },
  "brave-search": { src: bravesearchLogo },
  firecrawl: { src: firecrawlLogo },
  tavily: { src: tavilyLogo },
  context7: { src: context7Logo },
  "chrome-devtools": { src: chromeLogo },
  deepwiki: { src: deepwikiLogo },
  "microsoft-learn": { src: microsoftLogo },
  playwright: { src: playwrightLogo },
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

export function hasRecipeLogo(id: string): boolean {
  return Object.prototype.hasOwnProperty.call(RECIPE_LOGOS, id);
}

const TILE_COLORS = [
  "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200",
  "bg-orange-100 text-orange-800 dark:bg-orange-900 dark:text-orange-200",
  "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200",
  "bg-rose-100 text-rose-800 dark:bg-rose-900 dark:text-rose-200",
  "bg-violet-100 text-violet-800 dark:bg-violet-900 dark:text-violet-200",
  "bg-emerald-100 text-emerald-800 dark:bg-emerald-900 dark:text-emerald-200",
  "bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200",
  "bg-cyan-100 text-cyan-800 dark:bg-cyan-900 dark:text-cyan-200",
];

export function letterTile(name: string): {
  letter: string;
  className: string;
} {
  const normalized = name.trim();
  let hash = 0;
  for (const character of normalized) {
    hash = (Math.imul(hash, 31) + character.codePointAt(0)!) >>> 0;
  }
  return {
    letter: Array.from(normalized)[0]?.toUpperCase() || "?",
    className: TILE_COLORS[hash % TILE_COLORS.length],
  };
}

export const TEMPLATE_ICONS: Record<
  string,
  { icon: LucideIcon; className: string }
> = {
  coding: { icon: Code2, className: TILE_COLORS[0] },
  chinese: { icon: Languages, className: TILE_COLORS[5] },
  review: { icon: ScanSearch, className: TILE_COLORS[4] },
};

export function LibraryResourceIcon({
  recipeId,
  kind,
  name,
  className,
}: {
  recipeId?: string;
  kind: LibraryKind;
  name?: string;
  className?: string;
}) {
  const logo =
    recipeId && Object.prototype.hasOwnProperty.call(RECIPE_LOGOS, recipeId)
      ? RECIPE_LOGOS[recipeId]
      : undefined;
  const template =
    kind === "instructions" &&
    recipeId &&
    Object.prototype.hasOwnProperty.call(TEMPLATE_ICONS, recipeId)
      ? TEMPLATE_ICONS[recipeId]
      : undefined;
  const tile = name !== undefined ? letterTile(name) : undefined;
  const TemplateIcon = template?.icon;
  const Icon = kind === "mcp" ? Plug : kind === "skills" ? Puzzle : FileText;
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex h-10 w-10 shrink-0 items-center justify-center rounded-ctl",
        logo
          ? "bg-pill text-ink-2"
          : template?.className ||
              (tile
                ? `${tile.className} text-lg font-semibold`
                : "bg-pill text-ink-2"),
        className,
      )}
    >
      {logo ? (
        <img
          src={logo.src}
          alt=""
          className={cn("h-6 w-6 object-contain", logo.invert && "dark:invert")}
        />
      ) : TemplateIcon ? (
        <TemplateIcon className="h-5 w-5" />
      ) : tile ? (
        tile.letter
      ) : (
        <Icon className="h-5 w-5" />
      )}
    </span>
  );
}
