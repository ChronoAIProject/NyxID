import { useEndpoints } from "@/hooks/use-endpoints";
import { usePublication } from "@/hooks/use-catalog-admin";
import type { DownstreamService } from "@/types/api";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
export function PublicationOperations({
  tool,
  disabled,
}: {
  tool: DownstreamService;
  disabled: boolean;
}) {
  const { data: operations = [] } = useEndpoints(tool.id);
  const mutation = usePublication();
  return (
    <div className="space-y-2">
      {operations.map((op) => (
        <div
          key={op.id}
          className="flex flex-wrap items-center justify-between gap-2 border-t border-border/50 pt-2 text-12"
        >
          <div>
            <span className="font-medium">{op.name}</span>
            <code className="ml-2 text-11 text-text-tertiary">
              {op.method} {op.path}
            </code>
          </div>
          <div className="flex items-center gap-2">
            <Badge
              variant={op.publication === "published" ? "success" : "secondary"}
            >
              {(op.publication ?? "published").replace(/^./, (s) =>
                s.toUpperCase(),
              )}
            </Badge>
            <Button
              disabled={disabled || mutation.isPending}
              onClick={() =>
                mutation.mutate(
                  {
                    serviceId: tool.id,
                    endpointId: op.id,
                    state:
                      op.publication === "published" ? "paused" : "published",
                  },
                  { onError: () => toast.error("Publication failed") },
                )
              }
            >
              {op.publication === "published" ? "Pause" : "Publish"}
            </Button>
          </div>
        </div>
      ))}
    </div>
  );
}
