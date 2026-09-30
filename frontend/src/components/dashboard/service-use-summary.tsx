import { Clock3 } from "lucide-react";
import {
  latestRecordedUse,
  recordedSourceLabel,
  callerLabel,
  outcomeLabel,
} from "@/lib/service-insights";
import { formatRelativeTime, cn } from "@/lib/utils";
import type { KeyInfo } from "@/types/keys";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import { ServiceOwnerAvatar } from "./service-owner-avatar";

export function ServiceUseSummary({
  connections,
  insights,
  onClick,
  expanded,
  controls,
  className,
  row = false,
}: {
  readonly connections: readonly KeyInfo[];
  readonly insights: ServiceInsightsState;
  readonly onClick: () => void;
  readonly expanded?: boolean;
  readonly controls?: string;
  readonly className?: string;
  readonly row?: boolean;
}) {
  const records = connections
    .flatMap((connection) => {
      const request = latestRecordedUse(
        insights.connections.get(connection.id)?.usage,
      );
      return request ? [{ connection, request }] : [];
    })
    .sort((a, b) => b.request.occurred_at.localeCompare(a.request.occurred_at));
  const latest = insights.status === "ready" ? records[0] : undefined;
  const recordedOrg =
    latest?.request.source?.kind === "org" &&
    latest.connection.credential_source?.type === "org" &&
    latest.connection.credential_source.org_id ===
      latest.request.source.owner_id
      ? latest.connection.credential_source
      : undefined;
  const ownOnly = connections.every(
    (connection) =>
      insights.connections.get(connection.id)?.usage?.activity.visibility ===
      "own_requests",
  );
  const allTracked =
    connections.length > 0 &&
    connections.every((connection) => {
      const activity = insights.connections.get(connection.id)?.usage?.activity;
      return (
        activity &&
        activity.tracking !== "unavailable" &&
        activity.visibility !== "unavailable"
      );
    });
  const unavailable =
    insights.status === "loading"
      ? "Loading…"
      : insights.status === "restricted"
        ? "History restricted"
        : insights.status === "error"
          ? "History couldn't load"
          : "Not recorded";
  const title = latest
    ? `${recordedSourceLabel(latest.request, latest.connection)} · ${latest.connection.slug}`
    : unavailable;
  const detail = latest
    ? `${callerLabel(latest.request.caller)}${latest.request.caller.app_name ? ` · ${latest.request.caller.app_name}` : ""} · ${formatRelativeTime(latest.request.occurred_at)}`
    : allTracked
      ? "No dispatched request in available history · 30d"
      : "Connection layer and caller unavailable";
  return (
    <button
      type="button"
      data-insight-view={row ? "requests" : undefined}
      data-connection-id={row ? connections[0]?.id : undefined}
      onClick={onClick}
      aria-expanded={expanded}
      aria-controls={controls}
      aria-label={
        row && connections.length === 1
          ? `Recent requests for ${connections[0]!.label}`
          : "View last used connection"
      }
      className={cn(
        "grid w-full grid-cols-[6rem_minmax(0,1fr)] items-start gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring",
        className,
      )}
    >
      <span className="pt-0.5 text-muted-foreground">
        {ownOnly ? "Your last use" : "Last used"}
      </span>
      <span className="min-w-0">
        <span
          className="flex min-h-5 min-w-0 items-center gap-1.5 font-medium"
          title={title}
        >
          {latest?.request.source ? (
            <ServiceOwnerAvatar
              type={latest.request.source.kind}
              name={
                recordedOrg?.org_name ??
                recordedSourceLabel(latest.request, latest.connection)
              }
              avatarUrl={recordedOrg?.avatar_url}
            />
          ) : (
            <Clock3 className="size-3.5 shrink-0 text-muted-foreground" />
          )}
          <span className="truncate">{title}</span>
        </span>
        <span
          className="mt-1 flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground"
          title={`${detail}${latest ? ` · ${outcomeLabel(latest.request.outcome)} · ${latest.request.occurred_at}` : ""}`}
        >
          {latest ? (
            <>
              <span className="truncate">
                {callerLabel(latest.request.caller)}
                {latest.request.caller.app_name
                  ? ` · ${latest.request.caller.app_name}`
                  : ""}
              </span>
              <span aria-hidden="true">·</span>
              <time className="shrink-0" dateTime={latest.request.occurred_at}>
                {formatRelativeTime(latest.request.occurred_at)}
              </time>
            </>
          ) : (
            <span className="truncate">{detail}</span>
          )}
        </span>
      </span>
    </button>
  );
}
