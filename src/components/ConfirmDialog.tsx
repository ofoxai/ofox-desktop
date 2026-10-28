import { useEffect, useRef, type ReactNode } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { AlertTriangle, Info, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ConfirmDialogProps {
  isOpen: boolean;
  title: string;
  message: string;
  /** Shown under the message, e.g. the files an action will touch. */
  details?: ReactNode;
  confirmText?: string;
  cancelText?: string;
  variant?: "destructive" | "info";
  zIndex?: "base" | "nested" | "alert" | "top";
  /** The confirmed action is running: both buttons are disabled and the
   *  dialog cannot be dismissed until it settles. */
  busy?: boolean;
  /** Why the confirmed action failed. Shown, and announced, inside the
   *  dialog; focus goes to Cancel, which the busy state had disabled. */
  error?: string;
  /** Where focus goes once the dialog has closed (default: the trigger). */
  onCloseAutoFocus?: (event: Event) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  isOpen,
  title,
  message,
  details,
  confirmText,
  cancelText,
  variant = "destructive",
  zIndex = "alert",
  busy = false,
  error,
  onCloseAutoFocus,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  const cancelButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (error) cancelButton.current?.focus();
  }, [error]);

  const IconComponent = variant === "info" ? Info : AlertTriangle;
  const iconClass =
    variant === "info" ? "h-5 w-5 text-brand" : "h-5 w-5 text-danger";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !busy) {
          onCancel();
        }
      }}
    >
      <DialogContent
        className="max-w-sm"
        zIndex={zIndex}
        onCloseAutoFocus={onCloseAutoFocus}
      >
        <DialogHeader className="gap-2 border-b-0 pb-0 pt-[18px]">
          <DialogTitle className="flex items-center gap-2">
            <IconComponent className={iconClass} />
            {title}
          </DialogTitle>
          <DialogDescription className="whitespace-pre-line">
            {message}
          </DialogDescription>
        </DialogHeader>
        {details && <div className="px-[18px] pt-3 text-ctl">{details}</div>}
        {error && (
          <p
            role="alert"
            className="break-words px-[18px] pt-3 text-ctl text-danger"
          >
            {error}
          </p>
        )}
        <DialogFooter className="border-t-0 pb-[18px] pt-4">
          <Button
            ref={cancelButton}
            variant="secondary"
            onClick={onCancel}
            disabled={busy}
          >
            {cancelText || t("common.cancel")}
          </Button>
          <Button
            variant={variant === "info" ? "primary" : "danger"}
            onClick={onConfirm}
            disabled={busy}
            aria-busy={busy || undefined}
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {confirmText || t("common.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
