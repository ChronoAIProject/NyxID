import { useSyncExternalStore } from "react";
import { Toaster as SonnerToaster } from "sonner";

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
  return (
    <SonnerToaster
      theme="dark"
      position={mobile ? "top-center" : "bottom-right"}
      gap={8}
      toastOptions={{
        classNames: {
          toast:
            "group !rounded-xl !border !text-[13px] !shadow-lg !shadow-primary/5 !gap-3 !p-3 !border-border !bg-card [&:not([data-type])]:!text-foreground [&_[data-icon]]:!flex [&_[data-icon]]:!h-[22px] [&_[data-icon]]:!w-[22px] [&_[data-icon]]:!shrink-0 [&_[data-icon]]:!items-center [&_[data-icon]]:!justify-center [&_[data-icon]]:!rounded-[6px] [&_[data-icon]>svg]:!h-3 [&_[data-icon]>svg]:!w-3 [&_[data-icon]]:!m-0",
          description: "!text-[12px] !mt-0",
          actionButton: "!bg-primary !text-primary-foreground",
          cancelButton: "!bg-muted !text-muted-foreground",
          success:
            "!border-success/30 !bg-success/[0.06] !text-success [&_[data-icon]]:!border [&_[data-icon]]:!border-success/20 [&_[data-icon]]:!bg-success/10 [&_[data-icon]>svg]:!text-success [&_[data-description]]:!text-success/70",
          error:
            "!border-destructive/30 !bg-destructive/[0.06] !text-destructive [&_[data-icon]]:!border [&_[data-icon]]:!border-destructive/20 [&_[data-icon]]:!bg-destructive/10 [&_[data-icon]>svg]:!text-destructive [&_[data-description]]:!text-destructive/70",
          info: "!border-nyx-500/30 !bg-nyx-500/[0.06] !text-nyx-secondary-400 [&_[data-icon]]:!border [&_[data-icon]]:!border-nyx-500/20 [&_[data-icon]]:!bg-nyx-500/10 [&_[data-icon]>svg]:!text-nyx-secondary-400 [&_[data-description]]:!text-nyx-secondary-400/70",
          warning:
            "!border-warning/30 !bg-warning/[0.06] !text-warning [&_[data-icon]]:!border [&_[data-icon]]:!border-warning/20 [&_[data-icon]]:!bg-warning/10 [&_[data-icon]>svg]:!text-warning [&_[data-description]]:!text-warning/70",
        },
      }}
    />
  );
}
