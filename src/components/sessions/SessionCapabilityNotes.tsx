import { useTranslation } from "react-i18next";
import { TOOL_META } from "@/config/toolMeta";
import type { ManagedSession } from "@/lib/api/sessions";

/** Static capability notes stay available without describing every tool as limited. */
export function SessionCapabilityNotes({
  sessions,
}: {
  sessions: ManagedSession[];
}) {
  const { t } = useTranslation();
  const byTool = new Map<string, Set<string>>();
  for (const session of sessions) {
    const notes = byTool.get(session.providerId) ?? new Set<string>();
    if (!session.capabilities.canReadMessages) notes.add("metadataOnly");
    if (session.capabilities.resumeUsesSessionModel) notes.add("piResumeModel");
    if (!session.capabilities.canTrash) notes.add("fileTrashUnavailable");
    if (notes.size) byTool.set(session.providerId, notes);
  }
  if (!byTool.size) return null;
  return (
    <details className="mb-3 text-note text-ink-muted">
      <summary className="w-fit cursor-pointer rounded focus-visible:outline-brand">
        {t("sessionCenter.capabilityNotes")}
      </summary>
      <ul className="mt-2 space-y-2 rounded-lg border border-line-2 bg-card px-3 py-2">
        {[...byTool].map(([provider, notes]) => (
          <li key={provider}>
            <span className="font-medium text-ink-2">
              {TOOL_META[provider]?.label ?? provider}
            </span>
            {[...notes].map((note) => (
              <p key={note}>{t(`sessionCenter.${note}`)}</p>
            ))}
          </li>
        ))}
      </ul>
    </details>
  );
}
