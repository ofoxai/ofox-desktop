import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  assistantErrorKey,
  DEFAULT_ASSISTANT_PREFERENCES,
  MAX_ASSISTANT_INSTRUCTIONS,
  type AssistantPreferences,
} from "@/lib/api/assistant";
import { cn } from "@/lib/utils";

interface AssistantSettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The saved settings, read again each time the dialog opens. */
  load: () => Promise<AssistantPreferences>;
  /** Save all of them; rejects with an assistant error code. */
  save: (preferences: AssistantPreferences) => Promise<unknown>;
}

/** One switch with its label and what it does. */
function SettingSwitch({
  label,
  hint,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  disabled: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <p id={`${id}-label`} className="text-ctl font-medium text-ink">
          {label}
        </p>
        <p id={`${id}-hint`} className="mt-0.5 text-note text-ink-muted">
          {hint}
        </p>
      </div>
      <Switch
        aria-labelledby={`${id}-label`}
        aria-describedby={`${id}-hint`}
        className="mt-0.5"
        checked={checked}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
    </div>
  );
}

/**
 * The assistant settings: custom instructions and the switches for generated
 * titles and finish notifications. Nothing is saved until Save, which saves
 * all three at once, so the instructions a message gets are never half typed.
 */
export function AssistantSettingsDialog({
  open,
  onOpenChange,
  load,
  save,
}: AssistantSettingsDialogProps) {
  const { t, i18n } = useTranslation();
  const ids = useId();
  // What the dialog edits; null until the saved settings are read.
  const [form, setForm] = useState<AssistantPreferences | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const loadNow = useRef(load);
  loadNow.current = load;

  useEffect(() => {
    if (!open) return;
    let current = true;
    setForm(null);
    setError(null);
    setSaving(false);
    loadNow.current().then(
      (saved) => current && setForm(saved),
      (reason) => current && setError(assistantErrorKey(reason)),
    );
    return () => {
      current = false;
    };
  }, [open]);

  const shown = form ?? DEFAULT_ASSISTANT_PREFERENCES;
  const editable = form !== null && !saving;
  const used = Array.from(shown.instructions).length;
  const tooLong = used > MAX_ASSISTANT_INSTRUCTIONS;
  const number = (value: number) => value.toLocaleString(i18n.language);
  const max = number(MAX_ASSISTANT_INSTRUCTIONS);
  const change = (patch: Partial<AssistantPreferences>) =>
    setForm((old) => (old ? { ...old, ...patch } : old));

  const submit = async () => {
    if (!form || tooLong || saving) return;
    setSaving(true);
    setError(null);
    try {
      await save(form);
      onOpenChange(false);
    } catch (reason) {
      setError(assistantErrorKey(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!saving) onOpenChange(next);
      }}
    >
      <DialogContent
        className="max-w-[520px]"
        zIndex="nested"
        aria-describedby={undefined}
      >
        <DialogHeader>
          <DialogTitle>{t("assistant.settings.title")}</DialogTitle>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-4 overflow-y-auto px-[18px] py-4">
          <div className="flex flex-col gap-1.5">
            <label
              htmlFor={`${ids}-instructions`}
              className="text-ctl font-medium text-ink"
            >
              {t("assistant.settings.instructions")}
            </label>
            <p id={`${ids}-hint`} className="text-note text-ink-muted">
              {t("assistant.settings.instructionsHint")}
            </p>
            <Textarea
              id={`${ids}-instructions`}
              rows={6}
              className="min-h-[140px] resize-y"
              placeholder={t("assistant.settings.instructionsPlaceholder")}
              value={shown.instructions}
              disabled={!editable}
              aria-invalid={tooLong || undefined}
              aria-describedby={`${ids}-hint ${ids}-count${tooLong ? ` ${ids}-limit` : ""}`}
              onChange={(event) => change({ instructions: event.target.value })}
            />
            <div className="flex items-start justify-between gap-3 text-note">
              <span id={`${ids}-limit`} className="text-danger">
                {tooLong &&
                  t("assistant.settings.instructionsTooLong", { max })}
              </span>
              <span
                id={`${ids}-count`}
                className={cn(
                  "shrink-0 tabular-nums",
                  tooLong ? "text-danger" : "text-ink-muted",
                )}
              >
                {t("assistant.settings.instructionsCount", {
                  used: number(used),
                  max,
                })}
              </span>
            </div>
          </div>
          <SettingSwitch
            label={t("assistant.settings.autoTitle")}
            hint={t("assistant.settings.autoTitleHint")}
            checked={shown.autoTitle}
            disabled={!editable}
            onCheckedChange={(autoTitle) => change({ autoTitle })}
          />
          <SettingSwitch
            label={t("assistant.settings.notifyOnFinish")}
            hint={t("assistant.settings.notifyOnFinishHint")}
            checked={shown.notifyOnFinish}
            disabled={!editable}
            onCheckedChange={(notifyOnFinish) => change({ notifyOnFinish })}
          />
          {!form && !error && (
            <p className="flex items-center gap-1.5 text-note text-ink-muted">
              <Loader2
                aria-hidden
                className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
              />
              {t("assistant.settings.loading")}
            </p>
          )}
          {error && (
            <p role="alert" className="break-words text-ctl text-danger">
              {t(error)}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button
            variant="secondary"
            disabled={saving}
            onClick={() => onOpenChange(false)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            disabled={!editable || tooLong}
            aria-busy={saving || undefined}
            onClick={() => void submit()}
          >
            {saving && (
              <Loader2
                aria-hidden
                className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
              />
            )}
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
