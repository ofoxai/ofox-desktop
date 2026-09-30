import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
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
import {
  Download,
  Loader2,
  RefreshCw,
  Check,
  ChevronsUpDown,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { FetchedModel } from "@/lib/api/model-fetch";

interface ModelSelectFromApiProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  fetchedModels: FetchedModel[];
  isLoading: boolean;
  onFetch: () => void;
  /** 只保留指定前缀的模型（如 ["openai"]），不传则不过滤 */
  vendorFilter?: string[];
  /** 排除指定前缀的模型（如 ["google", "anthropic"]） */
  vendorExclude?: string[];
}

export function ModelSelectFromApi({
  id,
  value,
  onChange,
  placeholder,
  fetchedModels,
  isLoading,
  onFetch,
  vendorFilter,
  vendorExclude,
}: ModelSelectFromApiProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  const filtered = useMemo(() => {
    if (!vendorFilter && !vendorExclude) return fetchedModels;
    return fetchedModels.filter((m) => {
      const slashIdx = m.id.indexOf("/");
      const vendor = slashIdx > 0 ? m.id.slice(0, slashIdx).toLowerCase() : "";
      if (
        vendorFilter &&
        vendor &&
        !vendorFilter.some((v) => v.toLowerCase() === vendor)
      )
        return false;
      if (
        vendorExclude &&
        vendor &&
        vendorExclude.some((v) => v.toLowerCase() === vendor)
      )
        return false;
      return true;
    });
  }, [fetchedModels, vendorFilter, vendorExclude]);

  const grouped = useMemo(() => {
    const map: Record<string, FetchedModel[]> = {};
    for (const model of filtered) {
      const slashIdx = model.id.indexOf("/");
      const vendor =
        slashIdx > 0 ? model.id.slice(0, slashIdx) : model.ownedBy || "Other";
      if (!map[vendor]) map[vendor] = [];
      map[vendor].push(model);
    }
    return map;
  }, [filtered]);

  const vendors = useMemo(() => Object.keys(grouped).sort(), [grouped]);

  // 有模型数据: 可搜索的下拉 + 刷新按钮
  if (filtered.length > 0) {
    return (
      <div className="flex gap-1">
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <Button
              id={id}
              variant="outline"
              role="combobox"
              aria-expanded={open}
              className="flex-1 justify-between font-normal"
            >
              <span className="truncate">
                {value ||
                  placeholder ||
                  t("providerForm.selectModelPlaceholder")}
              </span>
              <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            className="w-[--radix-popover-trigger-width] p-0 z-[200]"
            align="start"
          >
            <Command>
              <CommandInput
                placeholder={t("providerForm.searchModelPlaceholder", {
                  defaultValue: "搜索模型...",
                })}
              />
              <CommandList>
                <CommandEmpty>
                  {t("providerForm.noModelFound", {
                    defaultValue: "未找到匹配的模型",
                  })}
                </CommandEmpty>
                {vendors.map((vendor) => (
                  <CommandGroup key={vendor} heading={vendor}>
                    {grouped[vendor].map((model) => (
                      <CommandItem
                        key={model.id}
                        value={model.id}
                        onSelect={(v) => {
                          onChange(v);
                          setOpen(false);
                        }}
                      >
                        <Check
                          className={cn(
                            "mr-2 h-4 w-4",
                            value === model.id ? "opacity-100" : "opacity-0",
                          )}
                        />
                        {model.id}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                ))}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
        <Button
          variant="outline"
          size="icon"
          className="shrink-0"
          type="button"
          onClick={onFetch}
          disabled={isLoading}
          title={t("providerForm.fetchModels")}
        >
          <RefreshCw className={cn("h-4 w-4", isLoading && "animate-spin")} />
        </Button>
      </div>
    );
  }

  // 加载中: 禁用的按钮 + Spinner
  if (isLoading) {
    return (
      <Button
        variant="outline"
        className="w-full justify-start text-muted-foreground"
        disabled
      >
        <Loader2 className="h-4 w-4 mr-2 animate-spin" />
        {t("providerForm.fetchingModels")}
      </Button>
    );
  }

  // 无模型: 获取按钮
  return (
    <Button
      variant="outline"
      className="w-full justify-start text-muted-foreground"
      type="button"
      onClick={onFetch}
    >
      <Download className="h-4 w-4 mr-2" />
      {t("providerForm.fetchModels")}
    </Button>
  );
}
