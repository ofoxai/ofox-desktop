import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// The design tokens in tailwind.config.cjs, so `cn` resolves conflicts with
// them as it does with Tailwind's own scale. Without this `text-ctl` (a font
// size) reads as a text colour and is dropped beside `text-ink-ctl`.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ["ui", "nav", "ctl", "sub", "note", "micro", "title", "kpi"],
      radius: ["list", "card", "ctl", "pop", "dialog"],
      shadow: ["pop", "dialog"],
      ease: ["soft"],
      animate: ["pop-in", "fade", "dialog-in"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
