import { CreditCard } from "lucide-react";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  summarizeBilling,
  summarizeBillingDetail,
  summarizeBillingModel,
  credentialLabel,
} from "@/lib/service-insights";
import { connectionSource, connectionSourceLabel } from "@/lib/service-view";
import type { KeyInfo } from "@/types/keys";
import { ServiceAvatarStack } from "./service-avatar-stack";

export function ServiceBillingSummary({
  connections,
  insights,
  serviceName,
  onOpen,
}: {
  readonly connections: readonly KeyInfo[];
  readonly insights: ServiceInsightsState;
  readonly serviceName: string;
  readonly onOpen: (connectionId: string) => void;
}) {
  const sources = new Map<string, KeyInfo[]>();
  for (const connection of connections) {
    if (!connection.is_active) continue;
    const type = connectionSource(connection);
    const id =
      type === "org" && connection.credential_source?.type === "org"
        ? `org:${connection.credential_source.org_id}`
        : type;
    const rows = sources.get(id) ?? [];
    rows.push(connection);
    sources.set(id, rows);
  }
  const model = summarizeBillingModel(
    insights.status,
    [...sources.values()].flat().map((row) => insights.connections.get(row.id)),
  );
  return (
    <div className="flex h-6 min-w-0 items-center gap-2 text-xs">
      <span className="w-16 shrink-0 text-muted-foreground">Billing</span>
      {sources.size ? (
        <>
          <ServiceAvatarStack
            label={`Show billing sources for ${serviceName}`}
            items={[...sources].map(([id, rows]) => {
              const first = rows[0]!;
              const type = connectionSource(first);
              const source =
                type === "platform" ? "Platform" : connectionSourceLabel(first);
              const sourceInsights = rows.map((connection) =>
                insights.connections.get(connection.id),
              );
              const sourceModel = summarizeBillingModel(
                insights.status,
                sourceInsights,
              );
              const org =
                type === "org" && first.credential_source?.type === "org"
                  ? first.credential_source
                  : undefined;
              return {
                id,
                type,
                name: source,
                avatarUrl: org?.avatar_url,
                detail: sourceModel,
                description:
                  insights.status === "ready"
                    ? `${[...new Set(rows.map((row) => credentialLabel(row, insights.connections.get(row.id)?.billing)))].join(" / ")} · NyxID payer: ${summarizeBilling(sourceInsights)} · ${summarizeBillingDetail(sourceInsights)}`
                    : undefined,
                actionLabel: `Show ${source} billing for ${serviceName}`,
                onSelect: () => onOpen(first.id),
              };
            })}
          />
          <span className="min-w-0 truncate font-medium" title={model}>
            {model}
          </span>
        </>
      ) : (
        <span className="flex items-center gap-1.5 text-muted-foreground">
          <CreditCard className="size-3.5" aria-hidden="true" />
          No enabled connections
        </span>
      )}
    </div>
  );
}
