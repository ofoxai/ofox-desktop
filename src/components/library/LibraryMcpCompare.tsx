import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type McpComparison,
  type McpComparisonSubject,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { libraryCommandUnavailable } from "./libraryText";
import { MenuSelect } from "@/components/ui/menu-select";
export function LibraryMcpCompare({
  subjects,
  epoch,
}: {
  subjects: { name: string; subject: McpComparisonSubject }[];
  epoch: number;
}) {
  const { t } = useTranslation();
  const [left, setLeft] = useState("0"),
    [right, setRight] = useState("1");
  const [result, setResult] = useState<McpComparison>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const inFlight = useRef<number>();
  useEffect(
    () => () => {
      generation.current++;
    },
    [],
  );
  const change = (side: "left" | "right", value: string) => {
    generation.current++;
    inFlight.current = undefined;
    setBusy(false);
    setResult(undefined);
    setError("");
    if (side === "left") setLeft(value);
    else setRight(value);
  };
  async function compare(quiet = false) {
    if (busy || inFlight.current !== undefined) return;
    const request = ++generation.current;
    inFlight.current = request;
    if (!quiet) {
      setBusy(true);
      setError("");
      setResult(undefined);
    }
    try {
      const value = await libraryApi.compareMcp(
        subjects[Number(left)].subject,
        subjects[Number(right)].subject,
        epoch,
      );
      if (request === generation.current) setResult(value);
    } catch (reason) {
      if (request === generation.current) {
        setResult(undefined);
        setError(
          t(
            libraryCommandUnavailable(reason)
              ? "library.workbench2.unsupported"
              : "library.workbench2.sourceUnconfirmed",
          ),
        );
      }
    } finally {
      if (inFlight.current === request) inFlight.current = undefined;
      if (request === generation.current) setBusy(false);
    }
  }
  const validate = useRef(() => compare(true));
  validate.current = () => compare(true);
  useEffect(() => {
    if (!result) return;
    const timer = setInterval(() => void validate.current(), 5000);
    const focus = () => void validate.current();
    window.addEventListener("focus", focus);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", focus);
    };
  }, [!!result]);
  return (
    <div className="space-y-3">
      <p className="text-note text-ink-muted">
        {t("library.workbench2.compareHint")}
      </p>
      <div className="flex flex-wrap gap-2">
        <MenuSelect
          aria-label={t("library.workbench2.compareLeft")}
          value={left}
          onChange={(value) => change("left", value)}
          options={subjects.map((subject, index) => ({
            value: String(index),
            label: subject.name,
          }))}
        />
        <MenuSelect
          aria-label={t("library.workbench2.compareRight")}
          value={right}
          onChange={(value) => change("right", value)}
          options={subjects.map((subject, index) => ({
            value: String(index),
            label: subject.name,
          }))}
        />
        <Button
          disabled={busy || left === right || subjects.length < 2}
          onClick={() => void compare()}
        >
          {t("library.workbench2.compareRun")}
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-ctl text-warn">
          {error}
        </p>
      )}
      {result && (
        <div className="max-h-80 overflow-auto rounded-lg border border-line">
          {result.fields.map((field) => (
            <div
              key={JSON.stringify(field.path)}
              className="flex justify-between gap-3 border-b border-line p-2 text-note"
            >
              <code className="break-all">{field.path.join(".")}</code>
              <span
                className={
                  field.state === "equal"
                    ? "shrink-0 text-ink-muted"
                    : "shrink-0 text-warn"
                }
              >
                {t(`library.workbench2.${field.state}`)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
