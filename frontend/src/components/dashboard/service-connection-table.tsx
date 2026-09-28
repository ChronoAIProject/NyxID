import { Fragment, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowUpRight,
  ChevronDown,
  History,
  LockKeyhole,
  Settings2,
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
}: {
  readonly connection: KeyInfo;
}) {
  const editable = canEditConnection(key);
  const rows = [
    ["Created", formatDateTime(key.created_at)],
    ["Added via", key.source_app_name || "Not recorded"],
    ["Last caller", "Not reported"],
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
}: {
  readonly connections: readonly KeyInfo[];
  readonly serviceName: string;
  readonly renderActions?: (connection: KeyInfo) => ReactNode;
  readonly onViewHistory?: (connection: KeyInfo) => void;
}) {
  const [open, setOpen] = useState<{
    id: string;
    view: "details" | "history";
  } | null>(null);
  const [observedAt] = useState(Date.now);
  const toggle = (id: string, view: "details" | "history") =>
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
          <TableHead scope="col" className="w-[21%]">
            Connection / Slug
          </TableHead>
          <TableHead scope="col" className="w-[17%]">
            Classification
          </TableHead>
          <TableHead scope="col" className="w-[16%]">
            Status
          </TableHead>
          <TableHead scope="col" className="w-[25%]">
            Configuration
          </TableHead>
          <TableHead scope="col" className="w-[21%]">
            Activity
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {connections.map((key) => {
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
                  {org && source === "platform" && (
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      Platform credential
                    </p>
                  )}
                </TableCell>
                <TableCell>
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
                </TableCell>
                <TableCell>
                  <p
                    className="truncate text-[11px]"
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
                    ) : (
                      <ConnectionMetadata connection={key} />
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
