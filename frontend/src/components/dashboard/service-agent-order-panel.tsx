import { useRef, useState, type ReactNode } from "react";
import { Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { ServiceConnectionGroup } from "@/lib/service-groups";
import type { ServiceGroupOrder } from "@/hooks/use-service-group-order";
import { cn } from "@/lib/utils";

export function ServiceAgentOrderPanel({
  group,
  order,
  actions,
  className,
}: {
  readonly group: ServiceConnectionGroup;
  readonly order: ServiceGroupOrder;
  readonly actions: ReactNode;
  readonly className?: string;
}) {
  const [helpOpen, setHelpOpen] = useState(false);
  const helpTrigger = useRef<HTMLButtonElement>(null);
  const helpPointerType = useRef<string | null>(null);
  if (
    !group.id.startsWith("catalog:") ||
    (group.connections.length < 2 && order.groupId !== group.id)
  )
    return null;
  const enabled = group.connections.filter((key) => key.is_active);
  const http = enabled.filter((key) => key.service_type === "http");
  const preferred = http.find((key) => key.preference_rank === 1);
  const otherProtocols = group.connections.some(
    (key) => key.service_type !== "http",
  );
  const summary = order.unavailable
    ? "Saved agent order unknown"
    : order.savedOrder(group.id).length > 0
      ? "Saved order · no enabled HTTP preference"
      : "Default server discovery order";
  const readState = order.unavailable
    ? "Saving agent order requires the backend update."
    : order.readError
      ? "Agent order could not be loaded."
      : order.readPending
        ? "Loading agent order."
        : null;
  const stateInSummary = !preferred && !order.unavailable && Boolean(readState);
  const availability = readState && (
    <p
      role={order.readError ? "alert" : "status"}
      className={stateInSummary ? undefined : "text-11"}
    >
      {readState}
    </p>
  );
  return (
    <section
      aria-label={`Agent discovery order for ${group.name}`}
      className={cn(
        "space-y-2 bg-card px-4 py-3 text-12 text-muted-foreground",
        className,
      )}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <div
          className="min-w-0 space-y-1 break-words"
          data-service-order-summary
        >
          <div className="flex items-center gap-1">
            {stateInSummary ? (
              availability
            ) : (
              <p>
                {preferred ? (
                  <>
                    <span>Preferred in discovery:</span>{" "}
                    <span className="font-medium text-foreground">
                      {preferred.label}
                    </span>
                  </>
                ) : (
                  summary
                )}
              </p>
            )}
            <TooltipProvider>
              <Tooltip open={helpOpen} onOpenChange={setHelpOpen}>
                <TooltipTrigger asChild>
                  <Button
                    ref={helpTrigger}
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="size-7 shrink-0 p-0 [&_svg]:size-3.5"
                    aria-label="How discovery order works"
                    onPointerDown={(event) => {
                      helpPointerType.current = event.pointerType;
                      event.preventDefault();
                    }}
                    onClick={(event) => {
                      event.preventDefault();
                      setHelpOpen((open) =>
                        helpPointerType.current === "touch" ? !open : true,
                      );
                      helpPointerType.current = null;
                    }}
                  >
                    <Info className="size-3.5" aria-hidden="true" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent
                  collisionPadding={16}
                  onPointerDownOutside={(event) => {
                    const target = event.detail.originalEvent.target;
                    if (
                      target instanceof Node &&
                      helpTrigger.current?.contains(target)
                    )
                      event.preventDefault();
                  }}
                  data-service-order-help
                  className="max-w-[min(20rem,calc(100vw-2rem))] space-y-2 text-12 leading-relaxed"
                >
                  <ul className="space-y-2">
                    <li>Connections that match equally are shown in your order.</li>
                    <li>
                      The AI chooses which connection to use; this order is a
                      preference.
                    </li>
                    <li>
                      Agents see only the enabled HTTP connections they can access.
                    </li>
                  </ul>
                  {otherProtocols && (
                    <p>
                      Other protocol connections keep their saved positions but
                      are excluded from tool discovery.
                    </p>
                  )}
                  {(group.slug?.startsWith("llm-") ||
                    group.connections.some(
                      (key) =>
                        key.inference != null || key.service_category === "llm",
                    )) && (
                    <p>
                      Provider gateway routing is separate from this discovery
                      order.
                    </p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p>
            {enabled.length} enabled ·{" "}
            {group.connections.length - enabled.length} disabled
          </p>
        </div>
        <div
          role="group"
          aria-label={`Discovery order actions for ${group.name}`}
          className="flex flex-wrap items-center gap-2"
        >
          {actions}
        </div>
        {order.readError && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void order.retryRead()}
          >
            Retry reads
          </Button>
        )}
      </div>
      {!stateInSummary && availability}
    </section>
  );
}
