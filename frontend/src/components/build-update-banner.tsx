import { useState } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useBuildUpdates } from "@/hooks/use-build-updates";

export function BuildUpdateBanner({ ready, queryClient }: {
  readonly ready: boolean;
  readonly queryClient: QueryClient;
}) {
  const { buildId, available, refresh } = useBuildUpdates(ready, queryClient);
  const [dismissedBuildId, setDismissedBuildId] = useState<string>();
  const [applying, setApplying] = useState(false);
  const [failedBuildId, setFailedBuildId] = useState<string>();

  async function apply() {
    setApplying(true);
    const applied = await refresh();
    setFailedBuildId(applied ? undefined : buildId);
    setApplying(false);
  }

  if (!available || dismissedBuildId === buildId) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center px-4">
      <div role="status" aria-live="polite" className="pointer-events-auto flex max-w-xl items-center gap-3 rounded-xl border border-border bg-card p-3 text-12 text-foreground shadow-lg">
        <RefreshCw className="size-4 shrink-0 text-primary" aria-hidden="true" />
        <div className="min-w-0">
          <p className="font-medium">A new version of NyxID is ready.</p>
          <p className="text-11 text-muted-foreground">
            {failedBuildId === buildId ? "The update is still being deployed. Please try again shortly." : "You can keep working. Save your changes before refreshing."}
          </p>
        </div>
        <Button size="sm" variant="primary" isLoading={applying} onClick={() => void apply()}>
          Refresh
        </Button>
        <Button size="icon" variant="ghost" aria-label="Dismiss update banner" onClick={() => setDismissedBuildId(buildId)}>
          <X className="size-3" aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
