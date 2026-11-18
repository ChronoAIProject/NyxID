import { useSyncExternalStore } from "react";
import { useRouterState } from "@tanstack/react-router";
import { Toaster as SonnerToaster } from "sonner";
import { useResolvedTheme } from "@/hooks/use-theme";

const mql =
  typeof window !== "undefined"
    ? window.matchMedia("(max-width: 767px)")
    : null;

function useIsMobile() {
  return useSyncExternalStore(
    (cb) => {
      mql?.addEventListener("change", cb);
      return () => mql?.removeEventListener("change", cb);
    },
    () => mql?.matches ?? false,
    () => false,
  );
}

/* ── NyxID Toast ── */
export function Toaster() {
  const mobile = useIsMobile();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const atTop = pathname === "/assistant" || pathname.startsWith("/assistant/");
  return <ToastViewport position={mobile ? "top-center" : atTop ? "top-right" : "bottom-right"} />;
}

/** Shared viewport also used by isolated component/browser checks. */
export function ToastViewport({ position }: { readonly position: "top-center" | "top-right" | "bottom-right" }) {
  const theme = useResolvedTheme();
  return (
    <SonnerToaster
      theme={theme}
      position={position}
      gap={8}
      toastOptions={{
        classNames: {
          toast:
            "group !flex-wrap !rounded-xl !border !text-15 !shadow-lg !shadow-primary/5 !gap-3 !p-3 !border-border [&_[data-content]]:!min-w-0 [&_[data-content]]:!flex-[1_1_calc(100%_-_22px_-_var(--spacing)*3)] [&:not([data-type])]:!bg-card [&:not([data-type])]:!text-foreground [&_[data-icon]]:!flex [&_[data-icon]]:!h-[22px] [&_[data-icon]]:!w-[22px] [&_[data-icon]]:!shrink-0 [&_[data-icon]]:!items-center [&_[data-icon]]:!justify-center [&_[data-icon]]:!rounded-[6px] [&_[data-icon]>svg]:!h-3 [&_[data-icon]>svg]:!w-3 [&_[data-icon]]:!m-0",
          title: "!type-toast-title !text-foreground",
          description: "!type-toast-description !mt-0 !text-muted-foreground whitespace-pre-line break-words",
          actionButton: "!type-toast-action !min-h-control !h-auto !max-w-full !whitespace-normal !break-words !ml-0 !py-1.5 !bg-primary !text-primary-foreground",
          cancelButton: "!type-toast-action !min-h-control !h-auto !max-w-full !whitespace-normal !break-words !ml-auto !py-1.5 !bg-muted !text-muted-foreground",
          // Typed backgrounds mix the accent INTO the solid card color
          // (color-mix) instead of a low-alpha tint — page content must not
          // bleed through toasts.
          success:
            "!border-success/30 !bg-[color-mix(in_oklab,var(--color-success)_8%,var(--color-card))] !text-success [&_[data-icon]]:!border [&_[data-icon]]:!border-success/20 [&_[data-icon]]:!bg-success/10 [&_[data-icon]>svg]:!text-success",
          error:
            "!border-destructive/30 !bg-[color-mix(in_oklab,var(--color-destructive)_8%,var(--color-card))] !text-destructive [&_[data-icon]]:!border [&_[data-icon]]:!border-destructive/20 [&_[data-icon]]:!bg-destructive/10 [&_[data-icon]>svg]:!text-destructive",
          info: "!border-nyx-500/30 !bg-[color-mix(in_oklab,var(--color-nyx-500)_8%,var(--color-card))] !text-nyx-secondary-400 [&_[data-icon]]:!border [&_[data-icon]]:!border-nyx-500/20 [&_[data-icon]]:!bg-nyx-500/10 [&_[data-icon]>svg]:!text-nyx-secondary-400",
          warning:
            "!border-warning/30 !bg-[color-mix(in_oklab,var(--color-warning)_8%,var(--color-card))] !text-warning [&_[data-icon]]:!border [&_[data-icon]]:!border-warning/20 [&_[data-icon]]:!bg-warning/10 [&_[data-icon]>svg]:!text-warning",
        },
      }}
    />
  );
}
