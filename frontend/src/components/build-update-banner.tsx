import { useState } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useBuildUpdates } from "@/hooks/use-build-updates";
import { router } from "@/router";

export function BuildUpdateBanner({ ready, queryClient }: {
  readonly ready: boolean;
  readonly queryClient: QueryClient;
}) {
  const { available, assistantChanged, refresh } = useBuildUpdates(ready, queryClient);
  const pathname = useRouterState({ router, select: (state) => state.location.pathname });
  const [dismissed, setDismissed] = useState(false);
  const [applying, setApplying] = useState(false);
  const [failed, setFailed] = useState(false);

  async function apply() {
    setApplying(true);
    const applied = await refresh();
    setFailed(!applied);
    setApplying(false);
  }

  const assistantPage = pathname === "/assistant" || pathname.startsWith("/assistant/");
  if (!available || dismissed || (assistantPage && !assistantChanged)) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center px-4">
      <div role="status" aria-live="polite" className="pointer-events-auto flex max-w-xl items-center gap-3 rounded-xl border border-border bg-card p-3 text-12 text-foreground shadow-lg">
        <RefreshCw className="size-4 shrink-0 text-primary" aria-hidden="true" />
        <div className="min-w-0">
          <p className="font-medium">A new version of NyxID is ready.</p>
          <p className="text-11 text-muted-foreground">
            {failed ? "The update is still being deployed. Please try again shortly." : "You can keep working. Save your changes before refreshing."}
          </p>
        </div>
        <Button size="sm" variant="primary" isLoading={applying} onClick={() => void apply()}>
          Refresh
        </Button>
        <Button size="icon" variant="ghost" aria-label="Dismiss update banner" onClick={() => setDismissed(true)}>
          <X className="size-3" aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
