import { GitBranch } from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  poolFailoverLabel,
  poolFailoverSummary,
  poolStrategyLabel,
} from "@/lib/service-pool-display";
import type { ServicePool } from "@/schemas/pools";

export function ServicePoolSummary({
  pools,
  loading,
  incomplete,
  serviceName,
  expanded,
  contentId,
  onOpen,
}: {
  readonly pools: readonly ServicePool[];
  readonly loading: boolean;
  readonly incomplete: boolean;
  readonly serviceName: string;
  readonly expanded: boolean;
  readonly contentId: string;
  readonly onOpen: () => void;
}) {
  const first = pools[0];
  const name =
    first?.name ??
    (loading
      ? "Loading pools…"
      : incomplete
        ? "Pool access incomplete"
        : "Individual slugs");
  const failover = first
    ? poolFailoverSummary(pools)
    : loading
      ? "Loading…"
      : incomplete
        ? "Not confirmed"
        : "No pool";
  const strategy =
    first && pools.length === 1
      ? {
          priority: "Priority",
          weighted: "Weighted",
          round_robin: "Round-robin",
        }[first.strategy]
      : undefined;

  return (
    <TooltipProvider delayDuration={180}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="flex h-8 w-full min-w-0 flex-col rounded-sm text-left text-xs leading-4 focus-visible:outline-2 focus-visible:outline-ring"
            aria-label={`Show routing for ${serviceName}`}
            aria-expanded={expanded}
            aria-controls={contentId}
            onClick={onOpen}
          >
            <span className="flex w-full min-w-0 items-center gap-2">
              <span className="w-16 shrink-0 text-muted-foreground">Pool</span>
              <GitBranch
                className="size-3.5 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <span className="truncate font-medium">{name}</span>
              {pools.length > 1 && (
                <span className="shrink-0 text-muted-foreground">
                  +{pools.length - 1}
                </span>
              )}
              {strategy && (
                <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                  {strategy}
                </span>
              )}
            </span>
            <span className="flex w-full min-w-0 items-center gap-2">
              <span className="w-16 shrink-0 text-muted-foreground">
                Failover
              </span>
              <span className="min-w-0 truncate">{failover}</span>
            </span>
          </button>
        </TooltipTrigger>
        <TooltipContent
          side="top"
          collisionPadding={12}
          className="max-w-[min(22rem,calc(100vw-2rem))] space-y-2 break-words [overflow-wrap:anywhere]"
        >
          {pools.map((pool) => (
            <div key={pool.id}>
              <p className="font-medium">
                {pool.name} · {poolStrategyLabel(pool)}
              </p>
              <p>{poolFailoverLabel(pool)}</p>
              <code>/api/v1/proxy/s/{pool.slug}</code>
            </div>
          ))}
          <p className="text-muted-foreground">
            {first
              ? "Configured policy applies when calling the pool slug. Actual attempts depend on eligible members and the request. Individual connection slugs run directly."
              : loading
                ? "Loading saved pool membership."
                : incomplete
                  ? "Pool membership could not be fully checked."
                  : "No saved pool contains these connections. Individual connection slugs run directly."}
          </p>
          {first && incomplete && (
            <p className="text-muted-foreground">
              Showing known pools only; additional organization pools may
              require admin access.
            </p>
          )}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
