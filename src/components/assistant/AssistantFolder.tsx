import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function AssistantFolder({
  onChoose,
}: {
  onChoose: (folder?: string) => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const choose = async (folder?: string) => {
    setBusy(true);
    try {
      if (await (folder === undefined ? onChoose() : onChoose(folder)))
        setPath("");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2">
      <p className="w-full text-note text-ink-muted">
        {t("assistant.local.folderHint")}
      </p>
      <Button variant="outline" disabled={busy} onClick={() => void choose()}>
        {t("assistant.local.chooseFolder")}
      </Button>
      <Input
        className="min-w-48 flex-1"
        aria-label={t("assistant.local.folderPath")}
        placeholder={t("assistant.local.folderPath")}
        value={path}
        onChange={(e) => setPath(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && path.trim()) {
            e.preventDefault();
            void choose(path.trim());
          }
        }}
      />
      <Button
        variant="ghost"
        disabled={busy || !path.trim()}
        onClick={() => void choose(path.trim())}
      >
        {t("assistant.local.useFolder")}
      </Button>
    </div>
  );
}
