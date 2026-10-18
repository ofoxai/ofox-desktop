import { useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ExternalLink, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { LibraryKind } from "@/lib/api/library";
import { LibraryResourceIcon } from "./LibraryResourceIcon";
import {
  LIBRARY_RECOMMENDATIONS,
  type LibraryRecommendation,
} from "./libraryRecommendations";

export type LibraryRecommendationMatch =
  | { state: "exactLibrary"; resourceId: string }
  | { state: "nativeOther"; entryId: string }
  | { state: "none" };
export type LibraryRecommendationLocation = Exclude<
  LibraryRecommendationMatch,
  { state: "none" }
>;
export interface LibraryDiscoverProps {
  kind: LibraryKind;
  busy?: boolean;
  /** Backend-confirmed matches only. Absence does not mean a recipe is absent. */
  matches?: Readonly<Record<string, LibraryRecommendationMatch>>;
  onChoose: (recipe: LibraryRecommendation, trigger: HTMLElement) => void;
  onOpenSource?: (url: string) => void;
  onLocateExisting?: (match: LibraryRecommendationLocation) => void;
  searchInputRef?: RefObject<HTMLInputElement>;
}
export function LibraryDiscover({
  kind,
  busy = false,
  matches,
  onChoose,
  onOpenSource,
  onLocateExisting,
  searchInputRef,
}: LibraryDiscoverProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [expanded, setExpanded] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const recipes = LIBRARY_RECOMMENDATIONS.filter(
    (recipe) => recipe.kind === kind,
  );
  const categories = [
    ...new Set(recipes.flatMap((recipe) => recipe.category ?? [])),
  ];
  const selectedCategory =
    category === "all" ||
    categories.includes(category as (typeof categories)[number])
      ? category
      : "all";
  const needle = query.trim().toLocaleLowerCase();
  const visible = recipes.filter((recipe) => {
    if (selectedCategory !== "all" && recipe.category !== selectedCategory)
      return false;
    const name = recipe.titleKey ? t(recipe.titleKey) : recipe.name;
    return (
      !needle ||
      [
        name,
        t(recipe.descriptionKey),
        recipe.publisher,
        recipe.category
          ? t(`library.discover2.categories.${recipe.category}`)
          : "",
      ]
        .filter(Boolean)
        .join(" ")
        .toLocaleLowerCase()
        .includes(needle)
    );
  });

  return (
    <div className="space-y-3">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div className="relative min-w-0 flex-[1_1_14rem]">
          <Search
            aria-hidden
            className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-faint"
          />
          <Input
            ref={searchInputRef ?? searchRef}
            type="search"
            aria-label={t("library.interaction.searchDiscover")}
            placeholder={t("library.interaction.searchDiscover")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            className="pl-9"
          />
        </div>
        <Select value={selectedCategory} onValueChange={setCategory}>
          <SelectTrigger
            aria-label={t("library.discover2.category")}
            className="w-auto min-w-36"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("library.discover2.all")}</SelectItem>
            {categories.map((value) => (
              <SelectItem key={value} value={value}>
                {t(`library.discover2.categories.${value}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span role="status" className="text-note text-ink-muted">
          {t("library.discover2.results", { count: visible.length })}
        </span>
      </div>
      <TooltipProvider delayDuration={250}>
        <div className="grid min-w-0 gap-3 min-[700px]:grid-cols-2 min-[1200px]:grid-cols-3">
          {visible.map((recipe) => {
            const name = recipe.titleKey ? t(recipe.titleKey) : recipe.name;
            const match = matches?.[recipe.id];
            const exactMatch =
              match?.state === "exactLibrary" ? match : undefined;
            const nativeMatch =
              match?.state === "nativeOther" ? match : undefined;
            return (
              <article
                key={recipe.id}
                aria-label={name}
                className="flex min-w-0 flex-col gap-3 rounded-list border border-line bg-card p-4"
              >
                <div className="flex min-w-0 items-start gap-3">
                  <LibraryResourceIcon
                    kind={recipe.kind}
                    recipeId={recipe.id}
                  />
                  <div className="min-w-0 flex-1">
                    <h3 className="break-words text-ui font-medium text-ink">
                      {name}
                    </h3>
                    <p className="mt-0.5 flex flex-wrap gap-x-2 text-note text-ink-muted">
                      <span>{recipe.publisher}</span>
                      {recipe.runtimeKey && <span>{t(recipe.runtimeKey)}</span>}
                    </p>
                  </div>
                </div>
                <p className="line-clamp-2 text-ctl leading-relaxed text-ink-2">
                  {t(recipe.descriptionKey)}
                </p>
                <Collapsible
                  open={expanded === recipe.id}
                  onOpenChange={(open) => setExpanded(open ? recipe.id : null)}
                  className="mt-auto"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line-2 pt-2">
                    <CollapsibleTrigger asChild>
                      <Button type="button" variant="ghost" size="sm">
                        {t(
                          expanded === recipe.id
                            ? "library.discover2.closeDetails"
                            : "library.discover2.details",
                        )}
                        <ChevronDown
                          aria-hidden
                          className={`h-3 w-3 transition-transform ${expanded === recipe.id ? "rotate-180" : ""}`}
                        />
                      </Button>
                    </CollapsibleTrigger>
                    {exactMatch ? (
                      <Button
                        type="button"
                        variant="secondary"
                        disabled={busy || !onLocateExisting}
                        title={
                          !onLocateExisting
                            ? t("library.discover2.locationUnavailable")
                            : undefined
                        }
                        onClick={() => onLocateExisting?.(exactMatch)}
                      >
                        {t("library.discover2.viewExisting")}
                      </Button>
                    ) : (
                      <Button
                        type="button"
                        variant="secondary"
                        disabled={busy}
                        onClick={(event) =>
                          onChoose(recipe, event.currentTarget)
                        }
                      >
                        {t("library.discover.choose", { name })}
                      </Button>
                    )}
                  </div>
                  <CollapsibleContent className="space-y-3 border-t border-line-2 pt-3 text-note leading-relaxed">
                    <div className="flex flex-wrap gap-2 text-note">
                      <span className="rounded-full bg-pill px-2 py-0.5 text-ink-muted">
                        {t(
                          exactMatch
                            ? "library.discover2.collected"
                            : nativeMatch
                              ? "library.discover2.nativeOther"
                              : match?.state === "none"
                                ? "library.discover2.notCollected"
                                : "library.discover2.configurable",
                        )}
                      </span>
                      {recipe.authRequired && (
                        <span className="rounded-full bg-pill px-2 py-0.5 text-ink-muted">
                          {t("library.discover2.nativeAuth")}
                        </span>
                      )}
                      {recipe.referenceExample && (
                        <span className="rounded-full bg-pill px-2 py-0.5 text-ink-muted">
                          {t("library.discover.reference")}
                        </span>
                      )}
                    </div>
                    {recipe.preparationKey && (
                      <p className="text-ink-muted">
                        {t(recipe.preparationKey)}
                      </p>
                    )}
                    <dl className="space-y-2">
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
                      {recipe.licenseKey && (
                        <div>
                          <dt className="font-medium text-ink-2">
                            {t("library.discover2.licenseLabel")}
                          </dt>
                          <dd className="text-ink-muted">
                            {t(
                              `library.discover2.licenses.${recipe.licenseKey}`,
                            )}
                          </dd>
                        </div>
                      )}
                      {kind === "mcp" && (
                        <div>
                          <dt className="font-medium text-ink-2">
                            {t("library.discover2.checkLabel")}
                          </dt>
                          <dd className="text-ink-muted">
                            {t(
                              recipe.checkMode === "ofox"
                                ? "library.discover2.checkOfox"
                                : "library.discover2.checkNative",
                            )}{" "}
                            · {t("library.discover2.notTested")}
                          </dd>
                        </div>
                      )}
                    </dl>
                    {recipe.id === "github" && (
                      <div className="space-y-1 text-ink-muted">
                        <p className="font-medium text-ink-2">
                          {t("library.discover2.readOnly")}
                        </p>
                        <p>{t("library.discover2.credentialBoundary")}</p>
                        <p>{t("library.discover2.githubChoice")}</p>
                      </div>
                    )}
                    {kind === "skills" && (
                      <p className="text-ink-muted">
                        {t(
                          exactMatch
                            ? "library.workbench2.fixedSkill"
                            : "library.discover2.selectedSkillNote",
                        )}
                      </p>
                    )}
                    <div className="space-y-1.5">
                      <p className="font-medium text-ink-2">
                        {t("library.discover.compatible")}
                      </p>
                      <div className="flex flex-wrap items-center gap-1">
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
                      <p className="text-ink-muted">
                        {t("library.discover2.compatibilityNote")}
                      </p>
                    </div>
                    <p className="text-ink-muted">
                      {t("library.discover2.reviewedAt", {
                        date: recipe.reviewedAt,
                      })}
                    </p>
                    <p className="text-ink-faint">
                      {t("library.discover2.reviewBoundary")}
                    </p>
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      {recipe.sourceUrl ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          disabled={busy || !onOpenSource}
                          onClick={() => onOpenSource?.(recipe.sourceUrl!)}
                        >
                          {t("library.discover.source")}
                          <ExternalLink aria-hidden className="h-3 w-3" />
                        </Button>
                      ) : (
                        <span className="text-ink-faint">
                          {t("library.discover.templateSource")}
                        </span>
                      )}
                    </div>
                  </CollapsibleContent>
                </Collapsible>
                {nativeMatch && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={busy || !onLocateExisting}
                    title={
                      !onLocateExisting
                        ? t("library.discover2.locationUnavailable")
                        : undefined
                    }
                    onClick={() => onLocateExisting?.(nativeMatch)}
                  >
                    {t("library.discover2.locateNative")}
                  </Button>
                )}
              </article>
            );
          })}
        </div>
      </TooltipProvider>
      {visible.length === 0 && (
        <div className="rounded-list border border-line bg-card p-5 text-center">
          <p className="text-ctl text-ink-muted">
            {t("library.discover2.empty")}
          </p>
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              setQuery("");
              setCategory("all");
              searchRef.current?.focus();
            }}
          >
            {t("library.discover2.reset")}
          </Button>
        </div>
      )}
      <p className="text-note text-ink-muted">
        {t("library.discover.noExecution")}
      </p>
    </div>
  );
}
