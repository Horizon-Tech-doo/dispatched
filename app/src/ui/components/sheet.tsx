import { X } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Dialog } from "radix-ui";
import type { ReactNode } from "react";
import { IconButton } from "./icon-button";

const EASE = [0.16, 1, 0.3, 1] as const;

/** A right-edge side sheet. The app stays visible behind it (no dimming). */
export function Sheet({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  width = 420,
  dismissible = true,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
  /** false: no close button, and Escape or a click outside does nothing (a task that must finish first). */
  dismissible?: boolean;
}) {
  const reduce = useReducedMotion();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <Dialog.Portal forceMount>
            <Dialog.Overlay className="fixed inset-0 z-40 bg-transparent" />
            <Dialog.Content
              asChild
              forceMount
              {...(description ? {} : { "aria-describedby": undefined })}
              {...(dismissible ? {} : { onEscapeKeyDown: (e: Event) => e.preventDefault(), onInteractOutside: (e: Event) => e.preventDefault() })}
            >
              <motion.div
                initial={reduce ? false : { x: 24, opacity: 0 }}
                animate={{ x: 0, opacity: 1 }}
                exit={reduce ? { opacity: 0 } : { x: 24, opacity: 0 }}
                transition={{ duration: reduce ? 0 : 0.15, ease: EASE }}
                style={{ width }}
                className="fixed inset-y-0 right-0 z-50 flex max-w-full flex-col border-l border-line bg-side text-fg shadow-float outline-none"
              >
                <div className="flex items-start gap-2 border-b border-line px-4 py-3">
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <Dialog.Title className="m-0 text-14 font-semibold">{title}</Dialog.Title>
                    {description ? (
                      <Dialog.Description className="m-0 text-12 text-muted">{description}</Dialog.Description>
                    ) : null}
                  </div>
                  {dismissible && (
                    <Dialog.Close asChild>
                      <IconButton label="Close"><X /></IconButton>
                    </Dialog.Close>
                  )}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
                {footer && <div className="flex justify-end gap-2 border-t border-line px-4 py-3">{footer}</div>}
              </motion.div>
            </Dialog.Content>
          </Dialog.Portal>
        )}
      </AnimatePresence>
    </Dialog.Root>
  );
}
