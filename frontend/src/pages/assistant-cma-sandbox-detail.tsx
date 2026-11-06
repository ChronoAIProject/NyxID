import { useParams } from "@tanstack/react-router";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCmaSandbox } from "@/hooks/use-nyxbot-cma-sandboxes";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { CmaSandboxCard } from "./assistant-cma-sandboxes";

export function AssistantCmaSandboxDetailPage() {
  const { sandboxId } = useParams({ strict: false }) as { sandboxId: string };
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const query = useCmaSandbox(sandboxId);
  if (!enabled)
    return <p role="status">Sandboxes are not enabled for this account.</p>;
  return (
    <div className="mx-auto w-full max-w-4xl">
      <div className="mb-6 flex items-center justify-between gap-3">
        <h1 className="text-24 font-semibold">
          {query.data?.label ?? "Sandbox"}
        </h1>
        <Button
          variant="outline"
          isLoading={query.isFetching}
          onClick={() => void query.refetch()}
        >
          <RefreshCw />
          Refresh
        </Button>
      </div>
      {query.isPending && <p role="status">Checking environment…</p>}
      {query.error && (
        <p role="alert" className="text-13 text-destructive">
          {query.error.message}
        </p>
      )}
      {query.data && (
        <CmaSandboxCard key={query.data.id} sandbox={query.data} detail />
      )}
    </div>
  );
}
