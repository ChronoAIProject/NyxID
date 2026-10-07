import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ServiceConnectionGroup } from "@/lib/service-groups";
import type { ServiceGroupOrder } from "@/hooks/use-service-group-order";

export function ServiceAgentOrderPanel({
  group,
  order,
  hasPool = false,
}: {
  readonly group: ServiceConnectionGroup;
  readonly order: ServiceGroupOrder;
  readonly hasPool?: boolean;
}) {
  if (group.connections.length < 2 || !group.id.startsWith("catalog:"))
    return null;
  const enabled = group.connections.filter((key) => key.is_active);
  const http = enabled.filter((key) => key.service_type === "http");
  const otherProtocols = group.connections.filter(
    (key) => key.service_type !== "http",
  );
  const preferred = http.find((key) => key.preference_rank === 1);
  const summary = preferred
    ? `Preferred in discovery: ${preferred.label}`
    : order.unavailable
      ? "Saved agent order unknown"
      : order.readError
        ? "Agent order could not be loaded"
        : order.readPending
          ? "Loading agent order"
          : order.savedOrder(group.id).length > 0
            ? "Saved order · no enabled HTTP preference"
            : "No agent order · default server discovery order";
  return (
    <section
      aria-label={`Agent discovery order for ${group.name}`}
      className="space-y-2 border-b border-border/50 px-4 py-3 text-12 text-muted-foreground"
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <p
          className="min-w-0 flex-1 break-words"
          role={
            order.readError ? "alert" : order.readPending ? "status" : undefined
          }
        >
          {summary} · {enabled.length} enabled ·{" "}
          {group.connections.length - enabled.length} disabled
        </p>
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
      {order.unavailable && (
        <p className="text-11">
          Saving agent order requires the backend update.
        </p>
      )}
      {preferred && order.readPending && (
        <p className="text-11">Loading agent order.</p>
      )}
      {preferred && order.readError && (
        <p className="text-11">Agent order could not be loaded.</p>
      )}
      <details className="group text-12">
        <summary className="flex w-fit list-none cursor-pointer items-center gap-1.5 rounded-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
          <ChevronRight
            className="size-3.5 transition-transform motion-reduce:transition-none group-open:rotate-90"
            aria-hidden="true"
          />
          How selection works
        </summary>
        <div className="mt-2 space-y-2">
          <p>
            The card's agent-key count is keys with access, not connections.
            Enabled HTTP connections are alternatives for the same service. Each
            has its own tool-name prefix, based on the connection slug shown in
            its row: <code>slug__…</code>.
          </p>
          {otherProtocols.length > 0 && (
            <p>
              Other protocols (
              {[
                ...new Set(
                  otherProtocols.map((key) => key.service_type.toUpperCase()),
                ),
              ].join(", ")}
              ) retain saved positions but are not part of connected MCP tool
              discovery. Moving them does not make them discoverable; their
              protocol-specific access is separate.
            </p>
          )}
          <p>
            Default server discovery order lists personal connections first,
            then organization connections in membership order, with newest
            connections first within each owner. At equal keyword relevance,
            your order rearranges only this service's existing discovery slots.
            Better matches still come first. This is advisory: the agent chooses
            which tool to call, and independent clients may use their own tools.
          </p>
          <p>
            The pills show your view. Restricted agent keys see dense ranks 1,
            2, … over their allowed connections and nodes in the same relative
            order. Not every enabled row is in every agent's scope. Enabled
            connections can appear in search even with unavailable credentials;{" "}
            <code>executable</code> reports credential and routing availability.
            Disabled connections are not discovered. These row badges do not
            verify providers.
          </p>
          <p>
            A named tool or exact connection slug runs that connection. Explicit
            service pools use their own priority or rotation and failover
            settings.{" "}
            <Link
              to="/keys"
              search={{ tab: "pools" }}
              className="text-primary-text hover:underline"
            >
              {hasPool ? "Manage Service Pools" : "Create a Service Pool"}
            </Link>
          </p>
          {(group.slug?.startsWith("llm-") ||
            group.connections.some(
              (key) => key.inference != null || key.service_category === "llm",
            )) && (
            <p>
              For services with a provider gateway,{" "}
              <code>/api/v1/llm/{"{provider}"}</code> and gateway models without{" "}
              <code>pool:</code> use one active connection chosen by NyxID in
              database order within the current owner tiers. This discovery
              order does not change that selection.
            </p>
          )}
        </div>
      </details>
    </section>
  );
}
