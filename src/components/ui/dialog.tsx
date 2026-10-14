import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { cn } from "@/lib/utils";
import { useInteractionActive } from "./interaction-scope";

const Dialog = DialogPrimitive.Root;

const DialogTrigger = DialogPrimitive.Trigger;

const DialogPortal = DialogPrimitive.Portal;

const DialogClose = DialogPrimitive.Close;

/**
 * The open dialog's content element. Radix Dialog keeps scroll (wheel/touch)
 * to its own content node, so a popover portaled to `document.body` from
 * inside a dialog cannot scroll its list; popovers read this and portal into
 * the dialog instead. `null` outside a dialog.
 */
const DialogContainerContext = React.createContext<HTMLElement | null>(null);

const useDialogContainer = () => React.useContext(DialogContainerContext);

const Z_INDEX = {
  base: "z-40",
  nested: "z-50",
  alert: "z-[60]",
  top: "z-[110]",
} as const;

const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay> & {
    zIndex?: keyof typeof Z_INDEX;
  }
>(({ className, zIndex = "base", ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      "fixed inset-0 animate-fade bg-black/[.28] backdrop-blur-[8px] backdrop-saturate-[1.1]",
      Z_INDEX[zIndex],
      className,
    )}
    {...props}
  />
));
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName;

const DialogContent = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> & {
    zIndex?: keyof typeof Z_INDEX;
    variant?: "default" | "fullscreen";
    overlayClassName?: string;
  }
>(
  (
    {
      className,
      children,
      zIndex = "base",
      variant = "default",
      overlayClassName,
      ...props
    },
    ref,
  ) => {
    const active = useInteractionActive();
    // Kept in state, not a ref, so popovers re-render with the node once it
    // exists and never portal to the body on the first open.
    const [container, setContainer] = React.useState<HTMLDivElement | null>(
      null,
    );
    const contentRef = React.useCallback(
      (node: HTMLDivElement | null) => {
        setContainer(node);
        if (typeof ref === "function") ref(node);
        else if (ref) ref.current = node;
      },
      [ref],
    );

    const variantClass = {
      default:
        "fixed left-1/2 top-1/2 flex flex-col w-full max-w-lg max-h-[90vh] -translate-x-1/2 -translate-y-1/2 rounded-dialog bg-card text-foreground shadow-dialog animate-dialog-in",
      fullscreen:
        "fixed inset-0 flex flex-col w-screen h-screen translate-x-0 translate-y-0 bg-background text-foreground p-0 rounded-none shadow-none",
    }[variant];

    if (!active) return null;
    return (
      <DialogPortal>
        <DialogOverlay zIndex={zIndex} className={overlayClassName} />
        <DialogPrimitive.Content
          ref={contentRef}
          className={cn(variantClass, Z_INDEX[zIndex], className)}
          onInteractOutside={(e) => {
            // 防止点击遮罩层关闭对话框
            e.preventDefault();
          }}
          {...props}
        >
          <DialogContainerContext.Provider value={container}>
            {children}
          </DialogContainerContext.Provider>
        </DialogPrimitive.Content>
      </DialogPortal>
    );
  },
);
DialogContent.displayName = DialogPrimitive.Content.displayName;

const DialogHeader = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn(
      "flex flex-col gap-1 text-left px-[18px] py-3.5 border-b border-line-2 flex-shrink-0",
      className,
    )}
    {...props}
  />
);
DialogHeader.displayName = "DialogHeader";

const DialogFooter = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn(
      "flex flex-col-reverse gap-2 sm:flex-row sm:justify-end sm:items-center px-[18px] py-3 border-t border-line-2 flex-shrink-0",
      className,
    )}
    {...props}
  />
);
DialogFooter.displayName = "DialogFooter";

const DialogTitle = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn("text-title font-semibold leading-tight", className)}
    {...props}
  />
));
DialogTitle.displayName = DialogPrimitive.Title.displayName;

const DialogDescription = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn("text-ctl leading-relaxed text-ink-muted", className)}
    {...props}
  />
));
DialogDescription.displayName = DialogPrimitive.Description.displayName;

export {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  DialogClose,
  DialogContainerContext,
  useDialogContainer,
};
