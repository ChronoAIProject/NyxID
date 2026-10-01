import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronRight, GitBranch } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useKeys } from "@/hooks/use-keys";
import { useServiceInsights } from "@/hooks/use-service-insights";
import { useServiceCardTransition } from "@/hooks/use-service-card-transition";
import {
  poolFailoverLabel,
  poolStrategyLabel,
} from "@/lib/service-pool-display";
import { cn } from "@/lib/utils";
import type { ServicePool } from "@/schemas/pools";
import { ServicePoolRoutingPanel } from "./service-pool-routing-panel";

export function ServicePoolCards({
  pools,
  actions,
  onEdit,
  initialOpenId,
}: {
  readonly pools: readonly ServicePool[];
  readonly actions: (pool: ServicePool) => ReactNode;
  readonly onEdit: (pool: ServicePool) => void;
  readonly initialOpenId?: string;
}) {
  const [open, setOpen] = useState<string | null>(initialOpenId ?? null);
  const linkedCard = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!initialOpenId) return;
    const frame = requestAnimationFrame(() => {
      linkedCard.current?.scrollIntoView({ block: "start", inline: "nearest" });
    });
    return () => cancelAnimationFrame(frame);
  }, [initialOpenId]);
  const transition = useServiceCardTransition();
  const keys = useKeys();
  const connections = keys.isError ? [] : (keys.data ?? []);
  const insights = useServiceInsights(connections);
  return (
    <div className="grid items-start gap-6 sm:grid-cols-2 xl:grid-cols-3">
      {pools.map((pool) => {
        const expanded = open === pool.id;
        return (
          <section
            key={pool.id}
            ref={pool.id === initialOpenId ? linkedCard : undefined}
            aria-label={`${pool.name} pool`}
            style={{
              viewTransitionName: `pool-card-${pool.id.replace(/[^a-zA-Z0-9-]/g, "")}`,
            }}
            className={cn(
              "min-w-0 scroll-mt-8 overflow-hidden rounded-xl border bg-card shadow-sm",
              expanded && "sm:col-span-2 xl:col-span-3",
            )}
          >
            <div className={cn("flex flex-col", !expanded && "h-64")}>
              <div className="flex flex-1 flex-col gap-3 p-4">
                <div className="flex items-start gap-3">
                  <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border bg-background/50">
                    <GitBranch className="size-5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <h3 className="truncate text-[15px] font-semibold">
                      {pool.name}
                    </h3>
                    <code
                      className="block truncate text-xs text-muted-foreground"
                      title={`/api/v1/proxy/s/${pool.slug}`}
                    >
                      {pool.slug}
                    </code>
                  </div>
                  {actions(pool)}
                </div>
                <p className="line-clamp-1 h-4 text-xs text-muted-foreground">
                  {pool.description ||
                    "One route across compatible connections"}
                </p>
                <div className="mt-auto space-y-1.5 text-xs">
                  <div className="flex items-center justify-between gap-2">
                    <span>{poolStrategyLabel(pool)}</span>
                    <Badge variant="secondary">
                      {pool.is_active ? "Enabled" : "Disabled"}
                    </Badge>
                  </div>
                  <p>{poolFailoverLabel(pool)}</p>
                  <p className="text-muted-foreground">
                    {pool.members.filter((member) => member.enabled).length} /{" "}
                    {pool.members.length} members enabled ·{" "}
                    {pool.member_contract === "ai_chat"
                      ? "AI chat"
                      : "Same API"}
                  </p>
                  <p className="text-muted-foreground">
                    Billing follows each attempted connection
                  </p>
                </div>
              </div>
              <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-t px-4">
                <Button
                  variant="ghost"
                  size="sm"
                  aria-expanded={expanded}
                  aria-controls={`pool-route-${pool.id}`}
                  onClick={(event) => {
                    const card = event.currentTarget.closest("section");
                    transition(
                      () => setOpen(expanded ? null : pool.id),
                      expanded ? undefined : card,
                    );
                  }}
                >
                  <ChevronRight
                    className={cn("size-3.5", expanded && "rotate-90")}
                  />
                  {expanded ? "Hide route" : "View route"}
                </Button>
                <Button variant="link" size="sm" onClick={() => onEdit(pool)}>
                  Configure
                </Button>
              </div>
            </div>
            {expanded && (
              <div id={`pool-route-${pool.id}`} className="border-t">
                <ServicePoolRoutingPanel
                  pool={pool}
                  connections={connections}
                  insights={insights}
                  onEdit={() => onEdit(pool)}
                />
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}
