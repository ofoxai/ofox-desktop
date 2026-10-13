import { useCallback, useMemo, useState, type UIEvent } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronsUpDown, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { cn } from "@/lib/utils";
import type { FetchedModel } from "@/lib/api/model-fetch";

// ---------------------------------------------------------------------------
// MultiModelPicker — searchable multi-select with select-all / clear shortcuts
// ---------------------------------------------------------------------------

// cmdk becomes noticeably slow when every catalog entry is mounted at once.
// Search the full catalog, but render it in small batches as the user scrolls.
const MODEL_PICKER_BATCH_SIZE = 80;

function filterPickerModels(
  models: FetchedModel[],
  query: string,
): FetchedModel[] {
  const search = query.trim().toLocaleLowerCase();
  if (!search) return models;
  return models.filter((model) =>
    `${model.id} ${model.name ?? ""} ${model.ownedBy ?? ""}`
      .toLocaleLowerCase()
      .includes(search),
  );
}

function groupPickerModels(
  models: FetchedModel[],
): Record<string, FetchedModel[]> {
  const grouped: Record<string, FetchedModel[]> = {};
  for (const model of models) {
    const slash = model.id.indexOf("/");
    const vendor =
      slash > 0 ? model.id.slice(0, slash) : model.ownedBy || "Other";
    (grouped[vendor] ||= []).push(model);
  }
  return grouped;
}

function usePickerResults(models: FetchedModel[]) {
  const [search, setSearch] = useState("");
  const [visibleCount, setVisibleCount] = useState(MODEL_PICKER_BATCH_SIZE);
  const filtered = useMemo(
    () => filterPickerModels(models, search),
    [models, search],
  );
  const visible = useMemo(
    () => filtered.slice(0, visibleCount),
    [filtered, visibleCount],
  );
  const grouped = useMemo(() => groupPickerModels(visible), [visible]);
  const vendors = useMemo(() => Object.keys(grouped).sort(), [grouped]);

  const updateSearch = useCallback((value: string) => {
    setSearch(value);
    setVisibleCount(MODEL_PICKER_BATCH_SIZE);
  }, []);
  const loadMore = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      const list = event.currentTarget;
      if (
        visibleCount < filtered.length &&
        list.scrollTop + list.clientHeight >= list.scrollHeight - 48
      ) {
        setVisibleCount((count) => count + MODEL_PICKER_BATCH_SIZE);
      }
    },
    [filtered.length, visibleCount],
  );

  return { search, updateSearch, filtered, grouped, vendors, loadMore };
}

export interface MultiModelPickerProps {
  id: string;
  selectedIds: string[];
  onChange: (ids: string[]) => void;
  models: FetchedModel[];
  loading: boolean;
  onFetch: (forceRefresh?: boolean) => void;
  portalContainer?: HTMLElement | null;
  /** Optional catalog summary; defaults to the count and shared-key notice. */
  summary?: string;
}

export function MultiModelPicker({
  id,
  selectedIds,
  onChange,
  models,
  loading,
  onFetch,
  portalContainer,
  summary,
}: MultiModelPickerProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
  const allSelected =
    models.length > 0 && models.every((model) => selected.has(model.id));
  const { search, updateSearch, filtered, grouped, vendors, loadMore } =
    usePickerResults(models);

  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next) {
        updateSearch("");
        if (models.length === 0 && !loading) onFetch();
      }
    },
    [loading, models.length, onFetch, updateSearch],
  );

  const toggle = useCallback(
    (id: string) => {
      if (selected.has(id)) {
        onChange(selectedIds.filter((selectedId) => selectedId !== id));
      } else {
        onChange([...selectedIds, id]);
      }
    },
    [onChange, selected, selectedIds],
  );

  return (
    <div className="space-y-2">
      <div className="flex gap-1">
        <Popover open={open} onOpenChange={handleOpenChange}>
          <PopoverTrigger asChild>
            <Button
              id={id}
              type="button"
              variant="outline"
              role="combobox"
              aria-expanded={open}
              className="flex-1 justify-between font-normal"
            >
              <span
                className={cn(
                  "truncate",
                  selectedIds.length === 0 && "text-muted-foreground",
                )}
              >
                {selectedIds.length > 0
                  ? t("modelPicker.selectedCount", {
                      count: selectedIds.length,
                    })
                  : t("modelPicker.selectAtLeastOne")}
              </span>
              <ChevronsUpDown className="ml-2 h-3.5 w-3.5 shrink-0 opacity-50" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            className="p-0"
            align="start"
            style={{ width: "var(--radix-popover-trigger-width)" }}
            container={portalContainer ?? undefined}
          >
            <Command shouldFilter={false}>
              <CommandInput
                placeholder={t("modelPicker.searchModelOrProvider")}
                className="h-9"
                value={search}
                onValueChange={updateSearch}
              />
              <CommandList className="max-h-72" onScroll={loadMore}>
                {loading ? (
                  <div className="flex items-center justify-center py-6 text-[12px] text-muted-foreground">
                    <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                    {t("modelPicker.loading")}
                  </div>
                ) : (
                  <>
                    {filtered.length === 0 && (
                      <CommandEmpty>
                        {t("modelPicker.noCompatibleModels")}
                      </CommandEmpty>
                    )}
                    {vendors.map((vendor) => (
                      <CommandGroup
                        key={vendor}
                        heading={
                          vendor === "Other"
                            ? t("modelPicker.otherVendor")
                            : vendor
                        }
                      >
                        {grouped[vendor].map((model) => (
                          <CommandItem
                            key={model.id}
                            value={`${model.id} ${model.name ?? ""} ${vendor}`}
                            onSelect={() => toggle(model.id)}
                          >
                            <span
                              className={cn(
                                "mr-2 flex h-4 w-4 shrink-0 items-center justify-center rounded border",
                                selected.has(model.id)
                                  ? "border-orange-500 bg-orange-500 text-white"
                                  : "border-border-default",
                              )}
                            >
                              {selected.has(model.id) && (
                                <Check className="h-3 w-3" />
                              )}
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-[12px] font-medium">
                                {model.name?.trim() || model.id}
                              </span>
                              <span className="block truncate text-[10px] text-muted-foreground">
                                {model.id}
                              </span>
                            </span>
                            {(model.supportedParameters ?? []).includes(
                              "reasoning",
                            ) && (
                              <span className="ml-2 rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] text-violet-600">
                                {t("modelPicker.reasoning")}
                              </span>
                            )}
                            {(model.inputModalities ?? []).includes(
                              "image",
                            ) && (
                              <span className="ml-1 rounded bg-sky-500/10 px-1.5 py-0.5 text-[9px] text-sky-600">
                                {t("modelPicker.vision")}
                              </span>
                            )}
                          </CommandItem>
                        ))}
                      </CommandGroup>
                    ))}
                  </>
                )}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </div>
      <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
        <span>
          {summary ??
            t("modelPicker.compatibleCountWithSharedKey", {
              count: models.length,
            })}
        </span>
        <span className="flex shrink-0 gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-[11px]"
            disabled={loading || models.length === 0 || allSelected}
            onClick={() => onChange(models.map((model) => model.id))}
          >
            {t("modelPicker.selectAllCompatible")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-[11px]"
            disabled={loading || selectedIds.length === 0}
            onClick={() => onChange([])}
          >
            {t("modelPicker.clearSelection")}
          </Button>
        </span>
      </div>
    </div>
  );
}

export interface ModelPickerProps {
  id: string;
  value: string;
  onChange: (v: string) => void;
  models: FetchedModel[];
  loading: boolean;
  onFetch: (forceRefresh?: boolean) => void;
  /**
   * Portal target for the popover. Defaults to `document.body` when omitted.
   * Pass the dialog's content node when this picker is rendered inside a
   * Radix Dialog, so the popover lands inside the dialog's scroll-lock
   * shard and the search/option list stays scrollable.
   */
  portalContainer?: HTMLElement | null;
}

export function ModelPicker({
  id,
  value,
  onChange,
  models,
  loading,
  onFetch,
  portalContainer,
}: ModelPickerProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const { search, updateSearch, filtered, grouped, vendors, loadMore } =
    usePickerResults(models);

  // Retry catalog loading when the picker opens with no available models.
  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next) {
        updateSearch("");
        if (models.length === 0 && !loading) onFetch();
      }
    },
    [models.length, loading, onFetch, updateSearch],
  );

  return (
    <div className="flex gap-1">
      <Popover open={open} onOpenChange={handleOpenChange}>
        <PopoverTrigger asChild>
          <Button
            id={id}
            type="button"
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="flex-1 justify-between font-normal"
          >
            <span className={cn("truncate", !value && "text-muted-foreground")}>
              {value || t("modelPicker.defaultRoute")}
            </span>
            <ChevronsUpDown className="ml-2 h-3.5 w-3.5 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          className="p-0"
          align="start"
          // Match the trigger width so the list never visually mismatches
          // its anchor (Radix exposes this via CSS var).
          style={{ width: "var(--radix-popover-trigger-width)" }}
          // Portal into the dialog's content node so this popover lives
          // inside Radix Dialog's scroll-lock shard — otherwise wheel
          // events on the model list get swallowed and the dropdown looks
          // "stuck" (see `ModelPickerProps.portalContainer` above).
          container={portalContainer ?? undefined}
        >
          <Command shouldFilter={false}>
            <CommandInput
              placeholder={t("modelPicker.searchModel")}
              className="h-9"
              value={search}
              onValueChange={updateSearch}
            />
            <CommandList onScroll={loadMore}>
              {loading ? (
                <div className="flex items-center justify-center py-6 text-[12px] text-muted-foreground">
                  <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                  {t("modelPicker.loading")}
                </div>
              ) : (
                <>
                  {filtered.length === 0 && (
                    <CommandEmpty>{t("modelPicker.noModels")}</CommandEmpty>
                  )}
                  {vendors.map((vendor) => (
                    <CommandGroup
                      key={vendor}
                      heading={
                        vendor === "Other"
                          ? t("modelPicker.otherVendor")
                          : vendor
                      }
                    >
                      {grouped[vendor].map((m) => (
                        <CommandItem
                          key={m.id}
                          value={m.id}
                          onSelect={() => {
                            onChange(m.id);
                            setOpen(false);
                          }}
                        >
                          <Check
                            className={cn(
                              "mr-2 h-3.5 w-3.5",
                              value === m.id ? "opacity-100" : "opacity-0",
                            )}
                          />
                          <span className="truncate">{m.id}</span>
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  ))}
                </>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  );
}
