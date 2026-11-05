import {
  forwardRef,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { assistantApi, type AssistantSkillChoice } from "@/lib/api/assistant";
import { Textarea } from "@/components/ui/textarea";

export function SkillChip({
  name,
  onRemove,
  onChoose,
  disabled,
}: {
  name: string;
  onRemove?: () => void;
  onChoose?: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <span className="inline-flex max-w-full items-center gap-1 rounded-ctl bg-brand-soft px-2 py-1 text-note text-brand-ink">
      {onChoose ? (
        <button
          type="button"
          disabled={disabled}
          className="min-w-0 truncate text-left"
          onClick={onChoose}
        >
          {t("assistant.skills.chip", { name })}
        </button>
      ) : (
        <span className="truncate">{t("assistant.skills.chip", { name })}</span>
      )}
      {onRemove && (
        <button
          type="button"
          disabled={disabled}
          aria-label={t("assistant.skills.remove", { name })}
          onClick={onRemove}
          className="shrink-0 rounded-ctl p-0.5 hover:bg-pill"
        >
          <X aria-hidden className="h-3 w-3" />
        </button>
      )}
    </span>
  );
}

type Props = Omit<ComponentProps<typeof Textarea>, "value" | "onChange"> & {
  value: string;
  onTextChange: (value: string) => void;
  skill?: string;
  onSkill: (skill: string | undefined) => void;
  /** Owner, generation and conversation fence every outstanding catalog read. */
  scope: string;
};
export const SkillPicker = forwardRef<HTMLTextAreaElement, Props>(
  function SkillPicker(
    {
      value,
      onTextChange,
      skill,
      onSkill,
      scope,
      onKeyDown,
      disabled,
      readOnly,
      ...props
    },
    forwarded,
  ) {
    const { t } = useTranslation();
    const id = useId();
    const input = useRef<HTMLTextAreaElement | null>(null);
    // Keep the parent's ref attached while earlier siblings run layout effects
    // (the attachment list returns focus synchronously after its last removal).
    const assignRef = useCallback(
      (node: HTMLTextAreaElement | null) => {
        input.current = node;
        if (typeof forwarded === "function") forwarded(node);
        else if (forwarded) forwarded.current = node;
      },
      [forwarded],
    );
    const [caret, setCaret] = useState(value.length);
    const [closedPrefix, setClosedPrefix] = useState<string | null>(null);
    const [choose, setChoose] = useState(false);
    const [items, setItems] = useState<AssistantSkillChoice[]>([]);
    const [loadedScope, setLoadedScope] = useState(scope);
    useEffect(() => {
      setChoose(false);
      setClosedPrefix(null);
      setActive(0);
    }, [scope]);
    const [loading, setLoading] = useState(false);
    const [failed, setFailed] = useState(false);
    const [active, setActive] = useState(0);
    const token = /^\/[^\s]*/.exec(value)?.[0];
    const prefix = token && caret <= token.length && caret > 0 ? token : null;
    const open =
      !disabled &&
      !readOnly &&
      (choose || Boolean(prefix && closedPrefix !== prefix));
    const query = choose ? "" : (prefix?.slice(1).toLowerCase() ?? "");
    const matches = (loadedScope === scope ? items : []).filter((item) =>
      `${item.name} ${item.description}`.toLowerCase().includes(query),
    );
    const index = Math.min(active, Math.max(0, matches.length - 1));
    useEffect(() => {
      if (!open) return;
      let current = true;
      setLoading(true);
      setFailed(false);
      setItems([]);
      void assistantApi
        .listSkills()
        .then((skills) => {
          if (current) {
            setItems(skills);
            setLoadedScope(scope);
          }
        })
        .catch(() => {
          if (current) setFailed(true);
        })
        .finally(() => {
          if (current) setLoading(false);
        });
      return () => {
        current = false;
      };
    }, [open, scope]);
    useEffect(() => {
      if (open && matches.length)
        document
          .getElementById(`${id}-${index}`)
          ?.scrollIntoView({ block: "nearest" });
    }, [open, id, index, matches.length]);
    const close = () => {
      setClosedPrefix(prefix);
      setChoose(false);
    };
    const pick = (name: string) => {
      onSkill(name);
      if (prefix) onTextChange(value.slice(prefix.length).replace(/^ /, ""));
      close();
      input.current?.focus();
    };
    const key = (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.nativeEvent.isComposing || event.keyCode === 229) {
        onKeyDown?.(event);
        return;
      }
      if (
        open &&
        ["ArrowDown", "ArrowUp", "Enter", "Escape"].includes(event.key) &&
        !event.shiftKey
      ) {
        event.preventDefault();
        if (event.key === "Escape") close();
        else if (event.key === "Enter") {
          if (matches[index]) pick(matches[index].name);
        } else
          setActive(
            (index + (event.key === "ArrowDown" ? 1 : -1) + matches.length) %
              Math.max(1, matches.length),
          );
        return;
      }
      if (
        event.key === "Backspace" &&
        skill &&
        event.currentTarget.selectionStart === 0 &&
        event.currentTarget.selectionEnd === 0
      ) {
        event.preventDefault();
        close();
        onSkill(undefined);
        return;
      }
      onKeyDown?.(event);
    };
    return (
      <div
        className="relative min-w-0 shrink-0"
        onKeyDown={(event) => {
          if (
            open &&
            event.key === "Escape" &&
            !event.nativeEvent.isComposing &&
            event.keyCode !== 229
          ) {
            event.preventDefault();
            close();
            input.current?.focus();
          }
        }}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) close();
        }}
      >
        {skill && (
          <div className="px-3 pt-2">
            <SkillChip
              name={skill}
              disabled={disabled || readOnly}
              onChoose={() => {
                setChoose(true);
                input.current?.focus();
              }}
              onRemove={() => {
                close();
                onSkill(undefined);
                input.current?.focus();
              }}
            />
          </div>
        )}
        <Textarea
          {...props}
          ref={assignRef}
          value={value}
          disabled={disabled}
          readOnly={readOnly}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          aria-activedescendant={
            open && matches.length ? `${id}-${index}` : undefined
          }
          onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
          onChange={(event) => {
            setCaret(event.target.selectionStart);
            setChoose(false);
            setClosedPrefix(null);
            setActive(0);
            onTextChange(event.target.value);
          }}
          onKeyDown={key}
        />
        {open && (
          <div
            className="absolute bottom-full left-2 right-2 z-30 mb-1 max-h-64 overflow-y-auto rounded-list border border-line bg-card p-1 shadow-lg"
            role="listbox"
            id={id}
            aria-label={t("assistant.skills.choose")}
          >
            {loading || failed || !matches.length ? (
              <p role="status" className="px-3 py-2 text-note text-ink-muted">
                {t(
                  loading
                    ? "assistant.loading"
                    : failed
                      ? "assistant.skills.unavailable"
                      : "assistant.skills.empty",
                )}
              </p>
            ) : (
              matches.map((item, i) => (
                <button
                  key={item.name}
                  id={`${id}-${i}`}
                  type="button"
                  role="option"
                  tabIndex={-1}
                  aria-selected={i === index}
                  className={`block w-full rounded-ctl px-3 py-2 text-left hover:bg-pill ${i === index ? "bg-pill" : ""}`}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => pick(item.name)}
                >
                  <span className="flex flex-wrap items-center gap-2 text-ctl text-ink">
                    <span className="break-all">{item.name}</span>
                    <span className="text-note text-ink-muted">
                      {t(`library.assistantTarget.${item.mode}`)}
                    </span>
                    {item.origin && item.origin !== "local" && (
                      <span className="text-note text-ink-muted">
                        {t(`library.assistantTarget.${item.origin}`)}
                      </span>
                    )}
                  </span>
                  <span className="block break-words text-note text-ink-muted">
                    {item.description}
                  </span>
                </button>
              ))
            )}
          </div>
        )}
      </div>
    );
  },
);
