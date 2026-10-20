import * as React from "react";
import { Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

export interface SearchInputProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    "value" | "onChange" | "type"
  > {
  value: string;
  onChange: (value: string) => void;
  /** Classes for the frame (width, margins); the field fills it. */
  className?: string;
}

/**
 * magpie's `.sess-filter`: a 26px field with a search icon whose border turns
 * brand while focused. A clear button shows while there is text, and Escape
 * clears it. Enclosing Radix layers may handle Escape first to dismiss their popup.
 */
export const SearchInput = React.forwardRef<HTMLInputElement, SearchInputProps>(
  (
    {
      value,
      onChange,
      className,
      placeholder,
      onKeyDown,
      "aria-label": ariaLabel,
      ...props
    },
    ref,
  ) => {
    const { t } = useTranslation();
    const inputRef = React.useRef<HTMLInputElement>(null);
    React.useImperativeHandle(ref, () => inputRef.current as HTMLInputElement);

    return (
      <div
        className={cn(
          "inline-flex h-[26px] w-[220px] max-w-full items-center rounded-ctl border border-line bg-card text-ctl text-ink transition-colors duration-150 ease-soft focus-within:border-brand",
          className,
        )}
      >
        <Search
          aria-hidden
          className="pointer-events-none ml-2 h-[13px] w-[13px] flex-none text-ink-faint"
        />
        <input
          ref={inputRef}
          type="search"
          value={value}
          placeholder={placeholder}
          aria-label={ariaLabel ?? placeholder}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            onKeyDown?.(event);
            if (event.defaultPrevented) return;
            if (event.key === "Escape" && value) {
              event.preventDefault();
              event.stopPropagation();
              onChange("");
            }
          }}
          className="h-full min-w-0 flex-1 bg-transparent px-1.5 text-ctl text-ink outline-none placeholder:text-ink-faint focus-visible:outline-none [&::-webkit-search-cancel-button]:appearance-none [&::-webkit-search-decoration]:appearance-none"
          {...props}
        />
        {value && (
          <button
            type="button"
            aria-label={t("common.clearSearch")}
            onClick={() => {
              onChange("");
              inputRef.current?.focus();
            }}
            className="mr-1 grid h-[18px] w-[18px] flex-none place-items-center rounded-[5px] text-ink-faint transition-colors duration-150 hover:bg-pill hover:text-ink"
          >
            <X aria-hidden className="h-3 w-3" strokeWidth={2} />
          </button>
        )}
      </div>
    );
  },
);
SearchInput.displayName = "SearchInput";
