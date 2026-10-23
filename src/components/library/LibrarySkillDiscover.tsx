import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw, Search, Settings2, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { MenuSelect } from "@/components/ui/menu-select";
import { SegmentedControl } from "@/components/ui/segmented";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  skillsApi,
  type DiscoverableSkill,
  type SkillRepo,
} from "@/lib/api/skills";
import type { SkillDefinition } from "@/lib/api/library";
import { collectedRepositorySkill } from "./skillIdentity";
import { LibraryDiscover } from "./LibraryDiscover";
import type { LibraryRecommendation } from "./libraryRecommendations";

const PAGE_SIZE = 40;
const REPOS_KEY = ["skillDiscoveryRepos"];
const repoId = (repo: SkillRepo) => `${repo.owner}/${repo.name}`;
type Source = "repos" | "skillssh" | "featured";

export function LibrarySkillDiscover({
  busy,
  enabled = true,
  rows,
  searchInputRef,
  onChoose,
  onChooseRecipe,
  onOpenSource,
}: {
  busy: boolean;
  enabled?: boolean;
  rows: SkillDefinition[];
  searchInputRef?: RefObject<HTMLInputElement>;
  onChoose: (skill: DiscoverableSkill) => void;
  onChooseRecipe: (recipe: LibraryRecommendation, trigger: HTMLElement) => void;
  onOpenSource: (url: string) => void;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [source, setSource] = useState<Source>("repos");
  const [search, setSearch] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [repository, setRepository] = useState("all");
  const [onlyNew, setOnlyNew] = useState(false);
  const [page, setPage] = useState(0);
  const manageTrigger = useRef<HTMLButtonElement>(null);
  const resolveTrigger = useRef<HTMLButtonElement | null>(null);
  const resolvedSelection = useRef(false);
  const [managing, setManaging] = useState(false);
  const [repoInput, setRepoInput] = useState("");
  const [branch, setBranch] = useState("main");
  const [resolving, setResolving] = useState<DiscoverableSkill | null>(null);
  useEffect(() => {
    if (enabled) return;
    setManaging(false);
    setResolving(null);
  }, [enabled]);
  const resolved = useQuery({
    queryKey: [
      "skillDiscoveryResolve",
      resolving?.repoOwner.toLowerCase(),
      resolving?.repoName.toLowerCase(),
    ],
    queryFn: () =>
      skillsApi.discoverRepository({
        owner: resolving!.repoOwner,
        name: resolving!.repoName,
        branch: "HEAD",
        enabled: true,
      }),
    enabled: !!resolving && enabled,
    staleTime: 300_000,
    retry: false,
  });
  const resolutionMatches = (resolved.data ?? []).filter(
    (skill) =>
      skill.directory.split("/").at(-1)?.toLocaleLowerCase() ===
        resolving?.directory.split("/").at(-1)?.toLocaleLowerCase() ||
      skill.name.toLocaleLowerCase() === resolving?.name.toLocaleLowerCase(),
  );
  const resolutionChoices = resolutionMatches.length
    ? resolutionMatches
    : (resolved.data ?? []);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const repos = useQuery({
    queryKey: REPOS_KEY,
    queryFn: skillsApi.getRepos,
    staleTime: 300_000,
    enabled,
    retry: false,
  });
  const configured = repos.data ?? [];
  const enabledRepos = configured.filter((repo) => repo.enabled);
  // Separate queries let a fast repository appear while another one is slow or fails.
  const catalogs = useQueries({
    queries: enabledRepos.map((repo) => ({
      queryKey: [
        "skillDiscoveryCatalog",
        repo.owner.toLowerCase(),
        repo.name.toLowerCase(),
        repo.branch,
      ],
      queryFn: () => skillsApi.discoverRepository(repo),
      enabled: enabled && source === "repos",
      staleTime: 300_000,
      gcTime: 1_800_000,
      retry: false,
    })),
  });
  useEffect(() => {
    if (source !== "skillssh") return;
    const timer = window.setTimeout(() => {
      setSubmitted(search.trim());
      setPage(0);
    }, 350);
    return () => window.clearTimeout(timer);
  }, [search, source]);
  const online = useQuery({
    queryKey: ["skillDiscoverySearch", submitted, page],
    queryFn: () =>
      skillsApi.searchSkillsSh(submitted, PAGE_SIZE, page * PAGE_SIZE),
    enabled:
      enabled &&
      source === "skillssh" &&
      submitted.length >= 2 &&
      submitted === search.trim(),
    staleTime: 300_000,
    retry: false,
  });
  const discovered = useMemo(() => {
    const all =
      source === "repos"
        ? catalogs.flatMap((catalog) => catalog.data ?? [])
        : (online.data?.skills ?? []).map((skill) => ({
            ...skill,
            key: `${skill.repoOwner}/${skill.repoName}:${skill.directory}`,
            description: "",
          }));
    const unique = new Map<string, DiscoverableSkill>();
    for (const skill of all) unique.set(skill.key, skill);
    return [...unique.values()].sort((a, b) => a.name.localeCompare(b.name));
  }, [source, catalogs, online.data]);
  const needle = search.trim().toLocaleLowerCase();
  const filtered = discovered.filter((skill) => {
    if (
      source === "repos" &&
      repository !== "all" &&
      `${skill.repoOwner}/${skill.repoName}` !== repository
    )
      return false;
    if (onlyNew && rows.some((row) => collectedRepositorySkill(row, skill.key)))
      return false;
    return (
      source !== "repos" ||
      !needle ||
      [
        skill.name,
        skill.description,
        skill.directory,
        `${skill.repoOwner}/${skill.repoName}`,
      ]
        .join(" ")
        .toLocaleLowerCase()
        .includes(needle)
    );
  });
  const pendingSearch = source === "skillssh" && submitted !== search.trim();
  const pageCount = Math.max(
    1,
    Math.ceil(
      (source === "repos" ? filtered.length : (online.data?.totalCount ?? 0)) /
        PAGE_SIZE,
    ),
  );
  const currentPage = Math.min(page, pageCount - 1);
  const visible = pendingSearch
    ? []
    : source === "repos"
      ? filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE)
      : filtered;
  const fetching =
    source === "repos"
      ? repos.isFetching || catalogs.some((catalog) => catalog.isFetching)
      : online.isFetching || pendingSearch;
  const failures = enabledRepos.filter((_, i) => catalogs[i]?.isError);
  const refresh = () => {
    if (source === "skillssh") {
      if (submitted.length >= 2) void online.refetch();
    } else {
      void repos.refetch();
      catalogs.forEach((catalog) => void catalog.refetch());
    }
  };
  const save = async (action: () => Promise<unknown>) => {
    setSaving(true);
    setSaveError(false);
    try {
      await action();
      await client.invalidateQueries({ queryKey: REPOS_KEY });
      setRepoInput("");
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <SegmentedControl<Source>
          aria-label={t("library.skillDiscover.source")}
          value={source}
          onChange={(value) => {
            setSource(value);
            setPage(0);
          }}
          options={[
            {
              value: "repos",
              label: t("library.skillDiscover.repos", {
                count: enabledRepos.length,
              }),
            },
            { value: "skillssh", label: "skills.sh" },
            { value: "featured", label: t("library.skillDiscover.featured") },
          ]}
        />
        {source !== "featured" && (
          <>
            <div className="relative min-w-48 flex-1">
              <Search
                aria-hidden
                className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-faint"
              />
              <Input
                ref={searchInputRef}
                type="search"
                value={search}
                onChange={(event) => {
                  setSearch(event.target.value);
                  if (source === "repos") setPage(0);
                }}
                aria-label={t("library.skillDiscover.search")}
                placeholder={t("library.skillDiscover.search")}
                className="pl-9"
              />
            </div>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("library.skillDiscover.refresh")}
              disabled={fetching}
              onClick={refresh}
            >
              <RefreshCw
                aria-hidden
                className={`h-4 w-4 ${fetching ? "animate-spin motion-reduce:animate-none" : ""}`}
              />
            </Button>
            <Button
              variant="secondary"
              ref={manageTrigger}
              onClick={() => setManaging(true)}
            >
              <Settings2 aria-hidden className="h-4 w-4" />
              {t("library.skillDiscover.manage")}
            </Button>
          </>
        )}
      </div>
      {source === "featured" ? (
        <LibraryDiscover
          kind="skills"
          busy={busy}
          searchInputRef={searchInputRef}
          onChoose={onChooseRecipe}
          onOpenSource={onOpenSource}
        />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3 text-note text-ink-muted">
            {source === "repos" && (
              <MenuSelect
                aria-label={t("library.skillDiscover.repoFilter")}
                value={repository}
                onChange={(value) => {
                  setRepository(value);
                  setPage(0);
                }}
                options={[
                  { value: "all", label: t("library.skillDiscover.allRepos") },
                  ...enabledRepos.map((repo) => ({
                    value: repoId(repo),
                    label: repoId(repo),
                  })),
                ]}
              />
            )}
            <label className="inline-flex items-center gap-2">
              <Checkbox
                checked={onlyNew}
                onCheckedChange={(value) => {
                  setOnlyNew(value === true);
                  setPage(0);
                }}
              />
              {t("library.skillDiscover.onlyNew")}
            </label>
            <span
              role="status"
              className="ml-auto inline-flex items-center gap-2"
            >
              {fetching && (
                <Loader2
                  aria-hidden
                  className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
                />
              )}
              {fetching
                ? t("library.skillDiscover.loading")
                : t("library.skillDiscover.results", {
                    count:
                      source === "repos"
                        ? filtered.length
                        : (online.data?.totalCount ?? 0),
                  })}
            </span>
          </div>
          {((repos.isError && source === "repos") ||
            (online.isError && source === "skillssh")) && (
            <p
              role="alert"
              className="flex items-center gap-2 text-note text-danger"
            >
              {t("library.skillDiscover.failed")}
              <Button variant="secondary" size="sm" onClick={refresh}>
                {t("common.retry")}
              </Button>
            </p>
          )}
          {source === "repos" && failures.length > 0 && (
            <p role="alert" className="text-note text-warn">
              {t("library.skillDiscover.partial", {
                repos: failures.map(repoId).join(", "),
              })}
              <Button
                variant="ghost"
                size="sm"
                onClick={() =>
                  failures.forEach(
                    (repo) =>
                      void client.invalidateQueries({
                        queryKey: [
                          "skillDiscoveryCatalog",
                          repo.owner.toLowerCase(),
                          repo.name.toLowerCase(),
                          repo.branch,
                        ],
                      }),
                  )
                }
              >
                {t("common.retry")}
              </Button>
            </p>
          )}
          <div
            aria-busy={fetching}
            className="overflow-hidden rounded-list border border-line bg-card"
          >
            {visible.map((skill) => {
              const collected = rows.some((row) =>
                collectedRepositorySkill(row, skill.key),
              );
              return (
                <article
                  key={skill.key}
                  aria-label={skill.name}
                  className="flex min-w-0 items-center gap-4 border-b border-line-2 px-4 py-3 last:border-b-0"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
                      <h3
                        className="truncate text-ui font-medium text-ink"
                        title={skill.name}
                      >
                        {skill.name}
                      </h3>
                      <button
                        type="button"
                        onClick={() =>
                          onOpenSource(
                            `https://github.com/${skill.repoOwner}/${skill.repoName}`,
                          )
                        }
                        className="max-w-full truncate rounded border border-line px-2 text-note text-ink-muted hover:text-ink"
                      >
                        {skill.repoOwner}/{skill.repoName}
                      </button>
                    </div>
                    {skill.description && (
                      <p className="mt-1 line-clamp-2 text-note text-ink-muted">
                        {skill.description}
                      </p>
                    )}
                    <p
                      className="mt-1 truncate font-mono text-note text-ink-faint"
                      title={skill.directory}
                    >
                      {skill.directory || "SKILL.md"}
                    </p>
                  </div>
                  <Button
                    variant="secondary"
                    disabled={busy || collected}
                    onClick={(event) => {
                      if (source === "skillssh") {
                        resolveTrigger.current = event.currentTarget;
                        resolvedSelection.current = false;
                        setResolving(skill);
                      } else onChoose(skill);
                    }}
                  >
                    {t(
                      collected
                        ? "library.skillDiscover.collected"
                        : "library.skillDiscover.choose",
                    )}
                  </Button>
                </article>
              );
            })}
            {!visible.length && (
              <p className="p-8 text-center text-note text-ink-muted">
                {fetching
                  ? t("library.skillDiscover.loading")
                  : source === "skillssh" && submitted.length < 2
                    ? t("library.skillDiscover.searchHint")
                    : source === "repos" && !enabledRepos.length
                      ? t("library.skillDiscover.noRepos")
                      : t("library.skillDiscover.empty")}
              </p>
            )}
          </div>
          {pageCount > 1 && (
            <div className="flex items-center justify-end gap-3 text-note text-ink-muted">
              <Button
                variant="secondary"
                disabled={page === 0 || fetching}
                onClick={() => setPage(Math.max(0, currentPage - 1))}
              >
                {t("library.skillDiscover.previous")}
              </Button>
              <span>
                {t("library.skillDiscover.page", {
                  page: currentPage + 1,
                  total: pageCount,
                })}
              </span>
              <Button
                variant="secondary"
                disabled={currentPage + 1 >= pageCount || fetching}
                onClick={() => setPage(currentPage + 1)}
              >
                {t("library.skillDiscover.next")}
              </Button>
            </div>
          )}
        </>
      )}
      <Dialog
        open={!!resolving}
        onOpenChange={(open) => {
          if (!open) setResolving(null);
        }}
      >
        <DialogContent
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (enabled && !resolvedSelection.current)
              resolveTrigger.current?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("library.skillDiscover.resolveTitle")}</DialogTitle>
            <DialogDescription>
              {t("library.skillDiscover.resolveHelp", {
                repo: resolving
                  ? `${resolving.repoOwner}/${resolving.repoName}`
                  : "",
              })}
            </DialogDescription>
          </DialogHeader>
          {resolved.isFetching && (
            <p
              role="status"
              className="flex items-center gap-2 text-note text-ink-muted"
            >
              <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
              {t("library.skillDiscover.loading")}
            </p>
          )}
          {resolved.isError && (
            <p role="alert" className="text-note text-danger">
              {t("library.skillDiscover.failed")}
              <Button
                variant="secondary"
                onClick={() => void resolved.refetch()}
              >
                {t("common.retry")}
              </Button>
            </p>
          )}
          <div className="max-h-80 space-y-2 overflow-y-auto">
            {resolutionChoices.map((skill) => (
              <div
                key={skill.key}
                className="flex items-center gap-3 rounded-ctl border border-line p-3"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-ui text-ink">{skill.name}</p>
                  <p className="break-all font-mono text-note text-ink-muted">
                    {skill.key.slice(skill.key.indexOf(":") + 1) || "SKILL.md"}
                  </p>
                </div>
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => {
                    resolvedSelection.current = true;
                    onChoose(skill);
                    setResolving(null);
                  }}
                >
                  {t("library.skillDiscover.confirmSource")}
                </Button>
              </div>
            ))}
          </div>
          {resolved.isSuccess && !resolutionChoices.length && (
            <p role="status" className="text-note text-ink-muted">
              {t("library.skillDiscover.empty")}
            </p>
          )}
          <div className="flex justify-end">
            <Button variant="secondary" onClick={() => setResolving(null)}>
              {t("common.cancel")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
      <Dialog
        open={managing}
        onOpenChange={(open) => {
          if (!saving) setManaging(open);
        }}
      >
        <DialogContent
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (enabled) manageTrigger.current?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("library.skillDiscover.manage")}</DialogTitle>
            <DialogDescription>
              {t("library.skillDiscover.repoHelp")}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-64 space-y-2 overflow-y-auto">
            {configured.map((repo) => (
              <div key={repoId(repo)} className="flex items-center gap-2">
                <label className="flex min-w-0 flex-1 items-center gap-2">
                  <Checkbox
                    checked={repo.enabled}
                    disabled={saving}
                    onCheckedChange={(checked) =>
                      void save(() =>
                        skillsApi.addRepo({
                          ...repo,
                          enabled: checked === true,
                        }),
                      )
                    }
                  />
                  <span className="truncate text-ctl">
                    {repoId(repo)}{" "}
                    <span className="text-ink-faint">{repo.branch}</span>
                  </span>
                </label>
                <Button
                  variant="ghost"
                  size="icon"
                  disabled={saving}
                  aria-label={t("library.skillDiscover.removeRepo", {
                    repo: repoId(repo),
                  })}
                  onClick={() =>
                    void save(() => skillsApi.removeRepo(repo.owner, repo.name))
                  }
                >
                  <Trash2 aria-hidden className="h-4 w-4" />
                </Button>
              </div>
            ))}
          </div>
          <form
            className="flex flex-wrap gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              const match = repoInput.trim().match(/^([\w-]+)\/([\w.-]+)$/);
              if (!match || !branch.trim()) {
                setSaveError(true);
                return;
              }
              void save(() =>
                skillsApi.addRepo({
                  owner: match[1],
                  name: match[2],
                  branch: branch.trim(),
                  enabled: true,
                }),
              );
            }}
          >
            <Input
              className="min-w-40 flex-1"
              aria-label={t("library.skillDiscover.repoInput")}
              placeholder="owner/repository"
              value={repoInput}
              onChange={(event) => setRepoInput(event.target.value)}
              disabled={saving}
            />
            <Input
              className="w-28"
              aria-label={t("library.skillDiscover.branch")}
              value={branch}
              onChange={(event) => setBranch(event.target.value)}
              disabled={saving}
            />
            <Button type="submit" disabled={saving}>
              {t("library.skillDiscover.addRepo")}
            </Button>
          </form>
          {saveError && (
            <p role="alert" className="text-note text-danger">
              {t("library.skillDiscover.repoFailed")}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              variant="secondary"
              disabled={saving}
              onClick={() => setManaging(false)}
            >
              {t("common.close")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
