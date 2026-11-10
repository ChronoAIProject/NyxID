import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Cable } from "lucide-react";
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
import { PoolStrategyIcon, ServicePoolIcon } from "./service-pool-icons";

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
  const { t } = useUiTranslation();
  const first = pools[0];
  const name =
    first?.name ??
    (loading
      ? t("Loading pools…")
      : incomplete
        ? t("Pool access incomplete")
        : t("Direct connections · no pool"));
  const failover = first
    ? poolFailoverSummary(pools)
    : loading
      ? t("Loading…")
      : incomplete
        ? t("Not confirmed")
        : t("No pool");
  const memberIds = new Set(
    pools.flatMap((pool) =>
      pool.members.map((member) => member.user_service_id),
    ),
  );
  const formats = [
    ...new Set(
      pools.map(
        (pool) =>
          t(({
            priority: "Priority",
            weighted: "Weighted",
            round_robin: "Round-robin",
          })[pool.strategy]),
      ),
    ),
  ].join(" / ");
  const config = first
    ? t(memberIds.size === 1 ? "{{count}} connection · {{formats}}" : "{{count}} connections · {{formats}}", { count: memberIds.size, formats })
    : loading
      ? t("Loading routing…")
      : incomplete
        ? t("Routing not confirmed")
        : t("Each connection uses its own slug");

  return (
    <TooltipProvider delayDuration={180}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="flex h-8 w-full min-w-0 flex-col rounded-sm text-left text-xs leading-4 focus-visible:outline-2 focus-visible:outline-ring"
            aria-label={t("Show routing for {{serviceName}}", { serviceName: serviceName })}
            aria-expanded={expanded}
            aria-controls={contentId}
            onClick={onOpen}
          >
            <span className="flex w-full min-w-0 items-center gap-2">
              {first &&
              pools.every((pool) => pool.strategy === first.strategy) ? (
                <PoolStrategyIcon
                  strategy={first.strategy}
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
              ) : first || loading || incomplete ? (
                <ServicePoolIcon className="size-3.5 shrink-0 text-muted-foreground" />
              ) : (
                <Cable
                  className="size-3.5 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
              )}
              <span className="truncate font-medium">
                {first ? config : name}
              </span>
              {pools.length > 1 && (
                <span className="ml-auto shrink-0 text-muted-foreground">
                  {pools.length}{t(" pools")}</span>
              )}
            </span>
            <span className="block w-full truncate pl-[22px] text-muted-foreground">
              {first
                ? t("{{value}}{{failover}}", { value: pools.length === 1 ? `${name} · ` : t("Failover · "), failover: failover })
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
              <p className="flex items-start gap-1.5 font-medium">
                <PoolStrategyIcon
                  strategy={pool.strategy}
                  className="mt-0.5 size-3.5 shrink-0"
                />
                <span>
                  {pool.name} · {poolStrategyLabel(pool)}
                </span>
              </p>
              <p>
                {pool.members.filter((member) => member.enabled).length}{t(" of")}{" "}
                {pool.members.length}{t(" connections enabled ·")}{" "}
                {poolFailoverLabel(pool)}
              </p>
              <code>/api/v1/proxy/s/{pool.slug}</code>
            </div>
          ))}
          <p className="text-muted-foreground">
            {first
              ? t("Configured policy applies when calling the pool slug. Actual attempts depend on eligible members and the request. Individual connection slugs run directly.")
              : loading
                ? t("Loading saved pool membership.")
                : incomplete
                  ? t("Pool membership could not be fully checked.")
                  : t("No saved pool contains these connections. Individual connection slugs run directly.")}
          </p>
          {first && incomplete && (
            <p className="text-muted-foreground">{t("Showing known pools only; additional organization pools may require admin access.")}</p>
          )}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
