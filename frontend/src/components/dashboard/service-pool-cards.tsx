import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useKeys } from "@/hooks/use-keys";
import { useServiceInsights } from "@/hooks/use-service-insights";
import { MotionConfig } from "motion/react";
import {
  poolFailoverLabel,
  poolStrategyLabel,
} from "@/lib/service-pool-display";
import { cn } from "@/lib/utils";
import type { ServicePool } from "@/schemas/pools";
import { ServicePoolRoutingPanel } from "./service-pool-routing-panel";
import { PoolStrategyIcon, ServicePoolIcon } from "./service-pool-icons";
import {
  CardReveal,
  Glide,
  MotionCard,
  MotionSurface,
} from "./service-card-motion";
import { useCardSequence } from "@/hooks/use-card-sequence";

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
  const keys = useKeys();
  const connections = keys.isError ? [] : (keys.data ?? []);
  const insights = useServiceInsights(connections);
  const sequence = useCardSequence({
    expanded: open,
    commit: setOpen,
    isRendered: (id) => pools.some((pool) => pool.id === id),
  });
  return (
    <MotionConfig reducedMotion="user">
      <div className="grid items-start gap-6 sm:grid-cols-2 xl:grid-cols-3">
        {pools.map((pool) => {
          const expanded = open === pool.id;
          return (
            <MotionCard
              key={pool.id}
              ref={pool.id === initialOpenId ? linkedCard : undefined}
              layoutKey={open}
              aria-label={`${pool.name} pool`}
              className={cn(
                "min-w-0 scroll-mt-8 overflow-hidden border bg-card shadow-sm",
                expanded && "sm:col-span-2 xl:col-span-3",
              )}
            >
              <MotionSurface
                className={cn("flex flex-col", !expanded && "h-64")}
              >
                <div className="flex flex-1 flex-col gap-3 p-4">
                  <Glide className="flex items-start gap-3">
                    <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border bg-background/50">
                      <ServicePoolIcon className="size-5" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <h3 className="truncate text-15 font-semibold">
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
                  </Glide>
                  <Glide>
                    <p className="line-clamp-1 h-4 text-xs text-muted-foreground">
                      {pool.description ||
                        "One route across compatible connections"}
                    </p>
                  </Glide>
                  <Glide className="mt-auto space-y-1.5 text-xs">
                    <div className="flex items-center justify-between gap-2">
                      <span className="flex items-center gap-1.5">
                        <PoolStrategyIcon
                          strategy={pool.strategy}
                          className="size-3.5 shrink-0"
                        />
                        {poolStrategyLabel(pool)}
                      </span>
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
                  </Glide>
                </div>
                <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-t px-4">
                  <Glide>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-expanded={expanded}
                      aria-controls={`pool-route-${pool.id}`}
                      onClick={(event) =>
                        sequence.request(
                          expanded ? null : pool.id,
                          event.currentTarget.closest("section"),
                        )
                      }
                    >
                      <ChevronRight
                        className={cn("size-3.5", expanded && "rotate-90")}
                      />
                      {expanded ? "Hide route" : "View route"}
                    </Button>
                  </Glide>
                  <Glide>
                    <Button
                      variant="link"
                      size="sm"
                      onClick={() => onEdit(pool)}
                    >
                      Configure
                    </Button>
                  </Glide>
                </div>
              </MotionSurface>
              <CardReveal
                open={sequence.isOpen(pool.id)}
                onClosed={sequence.onClosed}
              >
                <div id={`pool-route-${pool.id}`} className="border-t">
                  <ServicePoolRoutingPanel
                    pool={pool}
                    connections={connections}
                    insights={insights}
                    onEdit={() => onEdit(pool)}
                  />
                </div>
              </CardReveal>
            </MotionCard>
          );
        })}
      </div>
    </MotionConfig>
  );
}
