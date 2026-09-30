import { Fragment, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowUpRight,
  ChevronDown,
  History,
  LockKeyhole,
  Settings2,
  UsersRound,
  CreditCard,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { ServiceIcon } from "@/components/service-icon";
import { ServiceOwnerAvatar } from "./service-owner-avatar";
import { ServiceHistory } from "./service-history";
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table";
import { connectionSource } from "@/lib/service-view";
import { canEditConnection } from "@/lib/connection-access";
import { classifyConnection } from "@/lib/service-routing-preview";
import { cn, formatDate, formatDateTime } from "@/lib/utils";
import type { KeyInfo } from "@/types/keys";
import type { ServiceInsight } from "@/schemas/service-insights";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  ConnectionInsightPanel,
  type InsightPanel,
} from "./service-insight-panels";
import {
  billingAccountLabel,
  credentialLabel,
  rateLabel,
  accessCountLabel,
  providerBillingLabel,
  insightStatusLabel,
} from "@/lib/service-insights";
import { ServiceUseSummary } from "./service-use-summary";

const authNames: Record<string, string> = {
  bearer: "Bearer",
  api_key: "API key",
  oauth2: "OAuth 2.0",
  basic: "Basic",
  none: "No auth",
  node_managed: "Node credential",
};

function target(key: KeyInfo): string {
  if (key.credential_binding === "platform") return "Platform credential";
  if (key.service_type === "ssh")
    return key.ssh_host
      ? `${key.ssh_host}:${key.ssh_port ?? 22}`
      : "Target not reported";
  return key.endpoint_url || "Target not reported";
}

function ConnectionMetadata({
  connection: key,
  insight,
}: {
  readonly connection: KeyInfo;
  readonly insight?: ServiceInsight;
}) {
  const editable = canEditConnection(key);
  const rows = [
    ["Created", formatDateTime(key.created_at)],
    ["Added via", key.source_app_name || "Not recorded"],
    [
      "Last caller",
      insight?.usage?.activity.requests[0]?.caller.name || "Not recorded",
    ],
    [
      "Credential prepared",
      key.last_used_at ? formatDateTime(key.last_used_at) : "Not reported",
    ],
    ["Connection ID", key.id],
    [
      "Credential expires",
      key.expires_at ? formatDateTime(key.expires_at) : "Not reported",
    ],
    ...(editable
      ? [
          ["API target", target(key)],
          ["Node", key.node_id || "Direct"],
          ["Permissions", key.granted_scopes?.join(", ") || "Not reported"],
          [
            "Header names",
            key.default_request_headers
              ?.map((header) => header.name)
              .join(", ") || "None",
          ],
          ["WebSocket rules", String(key.ws_frame_injections?.length ?? 0)],
          ["User-Agent", key.custom_user_agent || "Client default"],
          ["OpenAPI", key.openapi_spec_url || "Not configured"],
          ["Recommended skills", key.recommended_skills?.join(", ") || "None"],
        ]
      : []),
  ];
  return (
    <dl className="grid gap-x-6 gap-y-3 px-1 py-2 text-[11px] sm:grid-cols-2 lg:grid-cols-3">
      {rows.map(([label, value]) => (
        <div key={label} className="min-w-0">
          <dt className="text-muted-foreground">{label}</dt>
          <dd className="mt-0.5 break-words [overflow-wrap:anywhere]">
            {value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function ServiceConnectionTable({
  connections,
  serviceName,
  renderActions,
  onViewHistory,
  insights: suppliedInsights,
}: {
  readonly connections: readonly KeyInfo[];
  readonly serviceName: string;
  readonly renderActions?: (connection: KeyInfo) => ReactNode;
  readonly onViewHistory?: (connection: KeyInfo) => void;
  readonly insights?: ServiceInsightsState;
}) {
  const [open, setOpen] = useState<{
    id: string;
    view: "details" | "history" | InsightPanel;
  } | null>(null);
  const [observedAt] = useState(Date.now);
  const insights = useServiceInsights(connections, suppliedInsights);
  const toggle = (id: string, view: "details" | "history" | InsightPanel) =>
    setOpen((current) =>
      current?.id === id && current.view === view ? null : { id, view },
    );

  return (
    <Table
      aria-label={`${serviceName} connections`}
      className="min-w-[840px] table-fixed"
      containerClassName="overscroll-x-contain"
    >
      <TableHeader>
        <TableRow>
          <TableHead scope="col" className="w-[22%]">
            Connection / Slug
          </TableHead>
          <TableHead scope="col" className="w-[17%]">
            Owner / Credential
          </TableHead>
          <TableHead scope="col" className="w-[22%]">
            Access &amp; requests
          </TableHead>
          <TableHead scope="col" className="w-[19%]">
            Billing
          </TableHead>
          <TableHead scope="col" className="w-[20%]">
            Configuration
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {connections.map((key) => {
          const insight = insights.connections.get(key.id);
          const billing = insight?.billing;
          const usage = insight?.usage;
          const source = connectionSource(key);
          const org =
            key.credential_source?.type === "org"
              ? key.credential_source
              : null;
          const owner =
            org?.org_name ??
            (source === "platform" ? "NyxID platform" : "Personal");
          const editable = canEditConnection(key);
          const readiness = classifyConnection(key, [], observedAt);
          const change =
            key.authorship?.last_change ?? key.authorship?.created_by;
          const changedAt = change?.at ?? key.created_at;
          const activity = key.authorship?.last_change ? "Changed" : "Created";
          const expanded = open?.id === key.id;
          const panelId = `connection-${key.id}-detail`;
          const route =
            key.node_id || key.has_node_binding
              ? `Node · ${key.node_status ?? "Unknown"}`
              : "Direct";
          const configCounts = [
            key.granted_scopes?.length
              ? `${key.granted_scopes.length} permissions`
              : null,
            key.default_request_headers?.length
              ? `${key.default_request_headers.length} headers`
              : null,
            key.ws_frame_injections?.length
              ? `${key.ws_frame_injections.length} WS rules`
              : null,
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            <Fragment key={key.id}>
              <TableRow
                data-service-connection-row={key.id}
                className={cn(
                  "[&>td]:align-top [&>td]:py-3",
                  expanded && "bg-muted/20",
                )}
              >
                <TableCell>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      className="flex size-5 shrink-0 items-center justify-center rounded-sm hover:bg-accent hover:text-primary focus-visible:outline-2 focus-visible:outline-ring"
                      onClick={() => toggle(key.id, "details")}
                      aria-expanded={expanded && open.view === "details"}
                      aria-controls={panelId}
                      aria-label={`Details for ${key.label} (${owner})`}
                      title="Show connection summary"
                    >
                      <ChevronDown
                        aria-hidden="true"
                        className={cn(
                          "size-3 shrink-0 -rotate-90 transition-transform motion-reduce:transition-none",
                          expanded && open.view === "details" && "rotate-0",
                        )}
                      />
                    </button>
                    <Link
                      to="/keys/$keyId"
                      params={{ keyId: key.id }}
                      aria-label={`View ${key.label} connection details (${owner})`}
                      className="inline-flex min-w-0 items-center gap-1 rounded-sm font-medium hover:text-primary hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                    >
                      <ServiceIcon
                        slug={key.catalog_service_slug ?? key.slug}
                        iconUrl={key.icon_url}
                        size="xs"
                      />
                      <span className="truncate" title={key.label}>
                        {key.label}
                      </span>
                      <ArrowUpRight
                        className="size-3 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                    </Link>
                  </div>
                  <code
                    className="mt-1 block truncate text-[11px] text-muted-foreground"
                    title={key.slug}
                  >
                    {key.slug}
                  </code>
                  <span className="mt-1 block text-[10px] uppercase tracking-wide text-muted-foreground">
                    {key.service_type}
                    {key.streaming_supported ? " · Streaming" : ""}
                    {key.websocket_supported ? " · WebSocket" : ""}
                  </span>
                  <div className="mt-2 space-y-1">
                    <Badge
                      variant={
                        readiness.state === "unavailable" && key.is_active
                          ? "warning"
                          : "secondary"
                      }
                    >
                      {readiness.reason}
                    </Badge>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {key.is_active ? "Enabled" : "Disabled"} ·{" "}
                      {key.status.replaceAll("_", " ")}
                    </p>
                    {key.expires_at && (
                      <p
                        className="mt-1 text-[11px] text-muted-foreground"
                        title={formatDateTime(key.expires_at)}
                      >
                        Expires {formatDate(key.expires_at)}
                      </p>
                    )}
                    {editable && renderActions?.(key)}
                  </div>
                </TableCell>
                <TableCell>
                  <div className="flex items-center gap-1.5">
                    <ServiceOwnerAvatar
                      type={org ? "org" : source}
                      name={owner}
                      avatarUrl={org?.avatar_url}
                    />
                    <span className="truncate font-medium" title={owner}>
                      {owner}
                    </span>
                  </div>
                  <p className="mt-1 text-[11px] capitalize text-muted-foreground">
                    {org
                      ? `Organization · ${org.role}`
                      : source === "platform"
                        ? "Platform managed"
                        : "Personal owner"}
                  </p>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {credentialLabel(key, billing)}
                  </p>
                </TableCell>
                <TableCell>
                  <button
                    type="button"
                    onClick={() => toggle(key.id, "access")}
                    aria-expanded={expanded && open.view === "access"}
                    aria-controls={panelId}
                    aria-label={`Agent key access for ${key.label}`}
                    data-insight-view="access"
                    data-connection-id={key.id}
                    className="inline-flex items-center gap-1.5 rounded-sm text-xs font-medium text-primary hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                  >
                    <UsersRound className="size-3.5 shrink-0" />
                    {usage
                      ? accessCountLabel(usage.access)
                      : insightStatusLabel(insights.status, "Access")}
                  </button>
                  {usage && (
                    <p className="mt-1 text-[10px] text-muted-foreground">
                      {usage.access.basis === "configuration"
                        ? `Configured scope${usage.access.incomplete ? " · partial inventory" : ""}`
                        : usage.access.visibility === "own_keys"
                          ? "Your keys with access"
                          : "Managed keys with access"}
                      {usage.access.keys.some(
                        (agent) => agent.credential_override,
                      ) &&
                        ` · ${usage.access.keys.filter((agent) => agent.credential_override).length} overrides`}
                    </p>
                  )}
                  {!!usage?.access.keys.length && (
                    <p
                      className="mt-1 max-w-52 truncate text-[11px]"
                      title={usage.access.keys
                        .map((agent) => agent.name)
                        .join(" · ")}
                    >
                      {usage.access.keys
                        .slice(0, 2)
                        .map((agent) => agent.name)
                        .join(" · ")}
                      {usage.access.keys.length > 2
                        ? ` +${usage.access.keys.length - 2}`
                        : ""}
                    </p>
                  )}
                  <ServiceUseSummary
                    row
                    connections={[key]}
                    insights={insights}
                    onClick={() => toggle(key.id, "requests")}
                    expanded={expanded && open.view === "requests"}
                    controls={panelId}
                    className="mt-3 grid-cols-1 gap-1"
                  />
                </TableCell>
                <TableCell>
                  <button
                    type="button"
                    onClick={() => toggle(key.id, "billing")}
                    aria-expanded={expanded && open.view === "billing"}
                    aria-controls={panelId}
                    aria-label={`Billing for ${key.label}`}
                    data-insight-view="billing"
                    data-connection-id={key.id}
                    className="block w-full rounded-sm text-left text-xs hover:text-primary focus-visible:outline-2 focus-visible:outline-ring"
                  >
                    <span className="mb-1 block text-[10px] text-muted-foreground">
                      {billing?.context === "configuration"
                        ? "Expected NyxID payer"
                        : billing?.status === "unavailable" ||
                            billing?.status === "restricted"
                          ? "Billing preview"
                          : "For your requests"}
                    </span>
                    <span className="flex items-start gap-1.5 font-medium">
                      <CreditCard className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                      {billing
                        ? billingAccountLabel(billing)
                        : insightStatusLabel(insights.status, "Billing")}
                    </span>
                    <span className="mt-1 block text-[11px] text-muted-foreground">
                      {billing?.status === "unavailable"
                        ? billing.notes[0]
                        : billing
                          ? rateLabel(billing)
                          : "Rate not reported"}
                    </span>
                    <span className="mt-1 block text-[11px] text-muted-foreground">
                      {providerBillingLabel(billing)}
                    </span>
                    <span className="mt-1.5 block text-[11px] text-primary">
                      View billing
                    </span>
                  </button>
                </TableCell>
                <TableCell>
                  {editable ? (
                    <>
                      <p
                        className="truncate font-mono text-[11px]"
                        title={target(key)}
                      >
                        {target(key)}
                      </p>
                      <p className="mt-1 truncate text-[11px] text-muted-foreground">
                        {authNames[key.auth_method] ?? key.auth_method} ·{" "}
                        {route}
                      </p>
                      {configCounts && (
                        <p
                          className="mt-1 truncate text-[11px] text-muted-foreground"
                          title={configCounts}
                        >
                          {configCounts}
                        </p>
                      )}
                      <Link
                        to="/keys/$keyId"
                        params={{ keyId: key.id }}
                        aria-label={`Configure ${key.label} (${owner})`}
                        className="mt-1.5 inline-flex items-center gap-1 text-[11px] text-primary hover:underline"
                      >
                        <Settings2 className="size-3" aria-hidden="true" />{" "}
                        Configure
                      </Link>
                    </>
                  ) : (
                    <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                      <LockKeyhole
                        className="size-3 shrink-0"
                        aria-hidden="true"
                      />
                      {key.auto_connected ? "Platform managed" : "Editors only"}
                    </p>
                  )}
                  <p
                    className="mt-2 truncate text-[11px]"
                    title={
                      change ? `${activity} by ${change.actor.name}` : undefined
                    }
                  >
                    {change ? `${activity} by ${change.actor.name}` : activity}
                  </p>
                  {changedAt && (
                    <time
                      dateTime={changedAt}
                      title={formatDateTime(changedAt)}
                      className="mt-1 block text-[11px] text-muted-foreground"
                    >
                      {formatDate(changedAt)}
                    </time>
                  )}
                  <button
                    type="button"
                    onClick={() =>
                      onViewHistory
                        ? onViewHistory(key)
                        : toggle(key.id, "history")
                    }
                    aria-label={`History for ${key.label} (${owner})`}
                    className="mt-1.5 inline-flex items-center gap-1 rounded-sm text-[11px] text-primary hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                  >
                    <History className="size-3" aria-hidden="true" /> History
                  </button>
                </TableCell>
              </TableRow>
              {expanded && (
                <TableRow
                  id={panelId}
                  className="bg-muted/10 hover:bg-muted/10"
                >
                  <TableCell colSpan={5} className="whitespace-normal">
                    {open.view === "history" ? (
                      <ServiceHistory serviceId={key.id} />
                    ) : open.view === "details" ? (
                      <ConnectionMetadata connection={key} insight={insight} />
                    ) : (
                      <ConnectionInsightPanel
                        key={`${key.id}:${open.view}`}
                        connection={key}
                        insight={insight}
                        view={open.view}
                        state={insights}
                      />
                    )}
                  </TableCell>
                </TableRow>
              )}
            </Fragment>
          );
        })}
      </TableBody>
    </Table>
  );
}
