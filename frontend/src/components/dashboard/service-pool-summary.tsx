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
        : "Direct connections · no pool");
  const failover = first
    ? poolFailoverSummary(pools)
    : loading
      ? "Loading…"
      : incomplete
        ? "Not confirmed"
        : "No pool";
  const memberIds = new Set(
    pools.flatMap((pool) =>
      pool.members.map((member) => member.user_service_id),
    ),
  );
  const formats = [
    ...new Set(
      pools.map(
        (pool) =>
          ({
            priority: "Priority",
            weighted: "Weighted",
            round_robin: "Round-robin",
          })[pool.strategy],
      ),
    ),
  ].join(" / ");
  const config = first
    ? `${memberIds.size} ${memberIds.size === 1 ? "connection" : "connections"} · ${formats}`
    : loading
      ? "Loading routing…"
      : incomplete
        ? "Routing not confirmed"
        : "Each connection uses its own slug";

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
              <GitBranch
                className="size-3.5 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <span className="truncate font-medium">
                {first ? config : name}
              </span>
              {pools.length > 1 && (
                <span className="ml-auto shrink-0 text-muted-foreground">
                  {pools.length} pools
                </span>
              )}
            </span>
            <span className="block w-full truncate pl-[22px] text-muted-foreground">
              {first
                ? `${pools.length === 1 ? `${name} · ` : "Failover · "}${failover}`
                : config}
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
              <p>
                {pool.members.filter((member) => member.enabled).length} of{" "}
                {pool.members.length} connections enabled ·{" "}
                {poolFailoverLabel(pool)}
              </p>
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
