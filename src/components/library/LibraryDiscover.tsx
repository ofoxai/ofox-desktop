import { useTranslation } from "react-i18next";
import { ExternalLink, FileText, Plug, Puzzle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { LibraryKind } from "@/lib/api/library";
import {
  LIBRARY_RECOMMENDATIONS,
  type LibraryRecommendation,
} from "./libraryRecommendations";
export interface LibraryDiscoverProps {
  kind: LibraryKind;
  busy?: boolean;
  onChoose: (recipe: LibraryRecommendation) => void;
  onOpenSource?: (url: string) => void;
}
export function LibraryDiscover({
  kind,
  busy = false,
  onChoose,
  onOpenSource,
}: LibraryDiscoverProps) {
  const { t } = useTranslation();
  const Icon = kind === "mcp" ? Plug : kind === "skills" ? Puzzle : FileText;
  return (
    <div className="space-y-3">
      <p className="text-ctl text-ink-muted">{t("library.discover.intro")}</p>
      <TooltipProvider delayDuration={250}>
        <div className="grid min-w-0 gap-3 md:grid-cols-2">
          {LIBRARY_RECOMMENDATIONS.filter((recipe) => recipe.kind === kind).map(
            (recipe) => {
              const name = recipe.titleKey ? t(recipe.titleKey) : recipe.name;
              return (
                <article
                  key={recipe.id}
                  aria-label={name}
                  className="flex min-w-0 flex-col gap-3 rounded-list border border-line bg-card p-4"
                >
                  <div className="flex min-w-0 items-start gap-3">
                    <div className="rounded-ctl bg-pill p-2 text-brand-ink">
                      <Icon aria-hidden className="h-4 w-4" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <h3 className="break-words text-ui font-medium text-ink">
                        {name}
                      </h3>
                      <p className="mt-1 text-ctl leading-relaxed text-ink-muted">
                        {t(recipe.descriptionKey)}
                      </p>
                    </div>
                  </div>
                  {recipe.referenceExample && (
                    <span className="self-start rounded-full bg-pill px-2 py-0.5 text-note text-ink-muted">
                      {t("library.discover.reference")}
                    </span>
                  )}
                  {(recipe.dependencyKeys.length > 0 ||
                    recipe.permissionKeys.length > 0) && (
                    <dl className="space-y-2 text-note leading-relaxed">
                      {recipe.dependencyKeys.length > 0 && (
                        <div>
                          <dt className="font-medium text-ink-2">
                            {t("library.discover.dependencies")}
                          </dt>
                          <dd className="text-ink-muted">
                            {recipe.dependencyKeys
                              .map((key) => t(key))
                              .join(" ")}
                          </dd>
                        </div>
                      )}
                      {recipe.permissionKeys.length > 0 && (
                        <div>
                          <dt className="font-medium text-ink-2">
                            {t("library.discover.permissionsLabel")}
                          </dt>
                          <dd className="text-ink-muted">
                            {recipe.permissionKeys
                              .map((key) => t(key))
                              .join(" ")}
                          </dd>
                        </div>
                      )}
                    </dl>
                  )}
                  {kind === "skills" && (
                    <p className="text-note text-ink-muted">
                      {t("library.discover.pinNote")}
                    </p>
                  )}
                  <div
                    aria-label={t("library.discover.compatible")}
                    className="mt-auto flex flex-wrap items-center gap-1"
                  >
                    {recipe.toolIds.map((tool) => (
                      <Tooltip key={tool}>
                        <TooltipTrigger asChild>
                          <span
                            tabIndex={0}
                            aria-label={TOOL_META[tool]?.label ?? tool}
                            className="rounded-ctl outline-none focus-visible:ring-2 focus-visible:ring-brand"
                          >
                            <ToolBadge
                              toolId={tool}
                              size={20}
                              variant="plain"
                            />
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          {TOOL_META[tool]?.label ?? tool}
                        </TooltipContent>
                      </Tooltip>
                    ))}
                  </div>
                  <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line-2 pt-3">
                    {recipe.sourceUrl ? (
                      <Button
                        type="button"
                        variant="ghost"
                        disabled={busy || !onOpenSource}
                        onClick={() => onOpenSource?.(recipe.sourceUrl!)}
                      >
                        {t("library.discover.source")}
                        <ExternalLink aria-hidden className="h-3 w-3" />
                      </Button>
                    ) : (
                      <span className="text-note text-ink-faint">
                        {t("library.discover.templateSource")}
                      </span>
                    )}
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={busy}
                      onClick={() => onChoose(recipe)}
                    >
                      {t("library.discover.choose", { name })}
                    </Button>
                  </div>
                </article>
              );
            },
          )}
        </div>
      </TooltipProvider>
      <p className="text-note text-ink-muted">
        {t("library.discover.noExecution")}
      </p>
    </div>
  );
}
