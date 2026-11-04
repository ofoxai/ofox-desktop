import { useTranslation } from "react-i18next";
import { useDelayedVisible } from "@/hooks/useDelayedVisible";
import { cn } from "@/lib/utils";

/**
 * The loading fox: Ofox's geometric fox with a face, hopping, tilting its
 * head and flicking its ears (keyframes in index.css, still under reduced
 * motion). For a page's or a block's first load only; buttons keep their
 * spinner. It shows after 200ms and stays at least 600ms, so a quick load
 * shows nothing and a slow one never blinks.
 */
export function FoxLoader({
  size = "page",
  label,
  className,
}: {
  /** "page": 60px with a progress line; "block": 36px for a card. */
  size?: "page" | "block";
  /** What is loading, e.g. 「正在读取用量…」; generic by default. */
  label?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const visible = useDelayedVisible(true);
  if (!visible) return null;
  const page = size === "page";
  const name = label ?? t("common.loadingFox");
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={name}
      className={cn(
        "flex flex-col items-center justify-center",
        page ? "py-16" : "py-8",
        className,
      )}
    >
      <FoxFace className={page ? "h-[60px] w-[60px]" : "h-9 w-9"} />
      <span
        aria-hidden
        className={cn(
          "fox-shadow mt-[3px] block rounded-[50%] bg-ink/15",
          page ? "h-1.5 w-[34px]" : "h-1 w-[22px]",
        )}
      />
      {page && (
        <span
          data-testid="fox-bar"
          aria-hidden
          className="mt-3 block h-[3px] w-[110px] overflow-hidden rounded-full bg-line"
        >
          <span className="fox-bar block h-full w-[40%] rounded-full bg-brand" />
        </span>
      )}
      {(page || label) && (
        <span className="mt-2.5 text-[12px] text-ink-muted">{name}</span>
      )}
    </div>
  );
}

/** The fox itself, drawn from the Ofox logo's ears and diamond face. */
function FoxFace({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 512 512"
      className={cn("fox-body block overflow-visible", className)}
    >
      <g className="fox-tilt">
        <g
          fill="#C0703C"
          stroke="#C0703C"
          strokeWidth={30}
          strokeLinejoin="round"
        >
          <g className="fox-ear-l" data-part="ear-l">
            <path d="M122 82 L214 150 L126 236 Z" />
            <path
              d="M140 118 L188 152 L144 196 Z"
              fill="#E9A57A"
              stroke="#E9A57A"
              strokeWidth={14}
            />
          </g>
          <g className="fox-ear-r" data-part="ear-r">
            <path d="M390 82 L298 150 L386 236 Z" />
            <path
              d="M372 118 L324 152 L368 196 Z"
              fill="#E9A57A"
              stroke="#E9A57A"
              strokeWidth={14}
            />
          </g>
          <path d="M258 184 L382 300 L262 424 L132 298 Z" />
        </g>
        <path
          d="M258 318 L318 362 L262 414 L202 360 Z"
          fill="#F6D9C4"
          stroke="#F6D9C4"
          strokeWidth={22}
          strokeLinejoin="round"
        />
        <g className="fox-eye" data-part="eye">
          <ellipse cx="214" cy="292" rx="17" ry="21" fill="#2B1A10" />
          <circle cx="220" cy="284" r="6" fill="#fff" />
        </g>
        <g className="fox-eye" data-part="eye">
          <ellipse cx="302" cy="292" rx="17" ry="21" fill="#2B1A10" />
          <circle cx="308" cy="284" r="6" fill="#fff" />
        </g>
        <ellipse
          cx="180"
          cy="334"
          rx="20"
          ry="11"
          fill="#F28B82"
          opacity={0.55}
        />
        <ellipse
          cx="336"
          cy="334"
          rx="20"
          ry="11"
          fill="#F28B82"
          opacity={0.55}
        />
        <path
          d="M248 352 Q260 346 272 352 Q262 366 260 366 Q258 366 248 352 Z"
          fill="#2B1A10"
        />
        <path
          d="M260 368 Q252 382 240 378 M260 368 Q268 382 280 378"
          fill="none"
          stroke="#2B1A10"
          strokeWidth={7}
          strokeLinecap="round"
        />
      </g>
    </svg>
  );
}
