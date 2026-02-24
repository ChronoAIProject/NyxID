import { Toaster as SonnerToaster } from "sonner";

/* ── VoidPortal Toast ── */
export function Toaster() {
  return (
    <SonnerToaster
      theme="dark"
      position="bottom-right"
      toastOptions={{
        classNames: {
          toast:
            "group border-border bg-surface text-foreground shadow-lg shadow-primary/5",
          description: "text-muted-foreground",
          actionButton: "bg-primary text-primary-foreground",
          cancelButton: "bg-muted text-muted-foreground",
          success: "border-success/30",
          error: "border-destructive/30",
        },
      }}
    />
  );
}
