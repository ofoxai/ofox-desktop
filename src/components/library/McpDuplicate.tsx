import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import type { McpDefinition } from "@/lib/api/library";
import { useDraftState, type DraftStore } from "./useDraftState";
export function McpDuplicate({
  store,
  initial,
  busy,
  onSave,
}: {
  store: DraftStore;
  initial: McpDefinition;
  busy: boolean;
  onSave: (id: string, name: string) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [id, setId] = useDraftState<string>(store, "copyId", () =>
    crypto.randomUUID(),
  );
  const [name, setName] = useDraftState(store, "copyName", () =>
    t("library.copyName", { name: initial.name }),
  );
  return (
    <div className="space-y-3">
      <p className="text-ctl text-ink-muted">
        {t("library.copyMcpDescription")}
      </p>
      <label className="block space-y-1 text-ctl">
        <span>{t("library.newMcpId")}</span>
        <Input
          value={id}
          onChange={(e) => setId(e.target.value)}
          disabled={busy}
          maxLength={256}
        />
      </label>
      <label className="block space-y-1 text-ctl">
        <span>{t("library.name")}</span>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={busy}
        />
      </label>
      <Button
        disabled={busy || !/^[A-Za-z0-9_.-]{1,256}$/.test(id) || !name.trim()}
        onClick={() => void onSave(id, name)}
      >
        {t("library.saveCopy")}
      </Button>
    </div>
  );
}
