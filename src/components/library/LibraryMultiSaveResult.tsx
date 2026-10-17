import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ResourceTarget } from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { LibraryMcpForm } from "./LibraryMcpForm";
import type { LibraryMcpDraft } from "./libraryRecommendations";
export interface MultiSaveResult {
  saved: { id: string; name: string }[];
  remaining: LibraryMcpDraft[];
  intent: "draft" | "apply";
  tools: string[];
}
export function LibraryMultiSaveResult({
  result,
  busy,
  targets,
  existingIds,
  onRetry,
  onPreview,
}: {
  result: MultiSaveResult;
  busy: boolean;
  targets: ResourceTarget[];
  existingIds: string[];
  onRetry: (draft: LibraryMcpDraft, index: number) => Promise<boolean>;
  onPreview: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<{
    index: number;
    draft: LibraryMcpDraft;
  }>();
  if (editing)
    return (
      <div className="space-y-3">
        <Button
          variant="ghost"
          disabled={busy}
          onClick={() => setEditing(undefined)}
        >
          {t("library.workbench2.back")}
        </Button>
        <LibraryMcpForm
          value={editing.draft}
          onChange={(draft) =>
            setEditing((previous) =>
              previous
                ? {
                    ...previous,
                    draft: { ...draft, identifier: previous.draft.identifier },
                  }
                : previous,
            )
          }
          identifier={editing.draft.identifier}
          onIdentifierChange={(identifier) =>
            setEditing((previous) =>
              previous
                ? { ...previous, draft: { ...previous.draft, identifier } }
                : previous,
            )
          }
          existingIdentifiers={existingIds}
          saveOnly
          targets={targets}
          selectedTools={[]}
          busy={busy}
          onSave={async (draft) => {
            if (await onRetry(draft, editing.index)) setEditing(undefined);
          }}
        />
      </div>
    );
  return (
    <div className="space-y-3">
      <div role="status" className="space-y-2">
        {result.saved.map((item) => (
          <p key={item.id} className="text-ctl">
            {item.name} · {t("library.workbench2.saved")}
          </p>
        ))}
        {result.remaining.map((draft, index) => (
          <div
            key={index}
            className="flex flex-wrap items-center justify-between gap-2"
          >
            <p className="text-ctl text-warn">
              {draft.name} · {t("library.workbench2.unfinished")}
            </p>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setEditing({ index, draft })}
            >
              {t("library.workbench2.reviewRemaining")}
            </Button>
          </div>
        ))}
      </div>
      {result.saved.length > 0 && result.intent === "apply" && (
        <Button disabled={busy} onClick={onPreview}>
          {t("library.workbench.previewChanges")}
        </Button>
      )}
    </div>
  );
}
