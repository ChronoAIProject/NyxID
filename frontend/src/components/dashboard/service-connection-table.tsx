import { Fragment, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowUpRight,
  ChevronDown,
  Clock3,
  History,
  LockKeyhole,
  Settings2,
  UsersRound,
  CreditCard,
  Info,
  type LucideIcon,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
import {
  cn,
  formatDate,
  formatDateTime,
  formatRelativeTime,
} from "@/lib/utils";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import type { ServicePool } from "@/schemas/pools";
import { poolStrategyLabel } from "@/lib/service-pool-display";
import { PoolStrategyIcon } from "./service-pool-icons";
import {
  connectionBillingCategory,
  connectionBillingLabels,
} from "@/lib/service-card-summary";
import type { ServiceInsight } from "@/schemas/service-insights";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  ConnectionInsightPanel,
  type InsightPanel,
} from "./service-insight-panels";
import { plainBilling } from "@/lib/billing-plain";
import {
  callerLabel,
  credentialLabel,
  accessCountLabel,
  latestRecordedUse,
  outcomeLabel,
  recordedSourceLabel,
  insightStatusLabel,
} from "@/lib/service-insights";
import { FadeIn } from "./service-card-motion";
import { useRevealMotion } from "@/hooks/use-card-sequence";

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
    <dl className="grid gap-x-6 gap-y-3 px-1 py-2 text-11 sm:grid-cols-2 lg:grid-cols-3">
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

/** A titled panel inside an opened connection, matching Billing and Agent keys. */
function PanelSection({
  icon: Icon,
  title,
  children,
}: {
  readonly icon: LucideIcon;
  readonly title: string;
  readonly children: ReactNode;
}) {
  return (
    <section className="space-y-3 p-3" aria-label={title}>
      <h4 className="inline-flex items-center gap-2 text-sm font-medium">
        <Icon className="size-4 text-primary-text" aria-hidden="true" /> {title}
      </h4>
      {children}
    </section>
  );
}

export function ServiceConnectionTable({
  connections,
  serviceName,
  renderActions,
  onViewHistory,
  insights: suppliedInsights,
  initialPanel = null,
  catalog,
  pools = [],
  onViewPool,
}: {
  readonly connections: readonly KeyInfo[];
  readonly serviceName: string;
  readonly renderActions?: (connection: KeyInfo) => ReactNode;
  readonly onViewHistory?: (connection: KeyInfo) => void;
  readonly insights?: ServiceInsightsState;
  readonly initialPanel?: { id: string; view: InsightPanel | "history" } | null;
  readonly catalog?: CatalogEntry;
  readonly pools?: readonly ServicePool[];
  readonly onViewPool?: (poolId: string) => void;
}) {
  const [open, setOpen] = useState<{
    id: string;
    view: "details" | "history" | InsightPanel;
  } | null>(initialPanel);
  const [observedAt] = useState(Date.now);
  const insights = useServiceInsights(connections, suppliedInsights);
  const reveal = useRevealMotion();
  const toggle = (id: string, view: "details" | "history" | InsightPanel) =>
    setOpen((current) =>
      current?.id === id && current.view === view ? null : { id, view },
    );

  return (
    <TooltipProvider delayDuration={200}>
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
              Billing &amp; usage
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
            const billingCategory =
              insights.status === "ready"
                ? connectionBillingCategory(key, billing, catalog)
                : "unknown";
            const memberships = pools.filter((pool) =>
              pool.members.some((member) => member.user_service_id === key.id),
            );
            const usage = insight?.usage;
            const latest = latestRecordedUse(usage);
            const useTracked =
              !!usage &&
              usage.activity.tracking !== "unavailable" &&
              usage.activity.visibility !== "unavailable";
            const overrideCount =
              usage?.access.keys.filter((agent) => agent.credential_override)
                .length ?? 0;
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
            const activity = key.authorship?.last_change
              ? "Changed"
              : "Created";
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
                    // The pool row and opened panel continue this entry, so no rule between them.
                    memberships.length > 0 && "border-b-0",
                    // The pool row is part of this entry, so they highlight together.
                    expanded
                      ? "border-b-0 bg-muted/20 hover:bg-muted/20"
                      : "[&:has(+tr[data-service-connection-pools]:hover)]:bg-overlay",
                  )}
                >
                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <button
                        type="button"
                        className="flex size-5 shrink-0 items-center justify-center rounded-sm hover:bg-accent hover:text-primary-text focus-visible:outline-2 focus-visible:outline-ring"
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
                        className="inline-flex min-w-0 items-center gap-1 rounded-sm font-medium hover:text-primary-text hover:underline focus-visible:outline-2 focus-visible:outline-ring"
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
                      <Badge
                        className="ml-auto shrink-0"
                        variant={
                          readiness.state === "unavailable" && key.is_active
                            ? "warning"
                            : "secondary"
                        }
                      >
                        {readiness.reason}
                      </Badge>
                    </div>
                    <p className="mt-1 flex min-w-0 items-center gap-1 text-11 text-muted-foreground">
                      <code className="truncate" title={key.slug}>
                        {key.slug}
                      </code>
                      <span className="shrink-0 text-10 uppercase tracking-wide">
                        · {key.service_type}
                        {key.streaming_supported ? " · Streaming" : ""}
                        {key.websocket_supported ? " · WebSocket" : ""}
                      </span>
                    </p>
                    <p className="mt-1 flex flex-wrap items-center gap-x-2 text-11 text-muted-foreground">
                      <span
                        title={
                          key.expires_at
                            ? formatDateTime(key.expires_at)
                            : undefined
                        }
                      >
                        {key.is_active ? "Enabled" : "Disabled"} ·{" "}
                        {key.status.replaceAll("_", " ")}
                        {key.expires_at
                          ? ` · Expires ${formatDate(key.expires_at)}`
                          : ""}
                      </span>
                      {editable && renderActions?.(key)}
                    </p>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <ServiceOwnerAvatar
                        type={org ? "org" : source}
                        name={owner}
                        avatarUrl={org?.avatar_url}
                      />
                      <span
                        className="truncate font-medium"
                        title={
                          org ? `${owner} · Organization · ${org.role}` : owner
                        }
                      >
                        {owner}
                      </span>
                    </div>
                    <p
                      className="mt-1 truncate text-11 text-muted-foreground"
                      title={credentialLabel(key, billing)}
                    >
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
                      className="inline-flex items-center gap-1.5 rounded-sm text-xs font-medium text-primary-text hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                    >
                      <UsersRound className="size-3.5 shrink-0" />
                      <span
                        className="truncate"
                        title={
                          usage
                            ? [
                                usage.access.basis === "configuration"
                                  ? `Configured scope${usage.access.incomplete ? " · partial inventory" : ""}`
                                  : usage.access.visibility === "own_keys"
                                    ? "Your keys with access"
                                    : "Managed keys with access",
                                ...usage.access.keys.map((agent) => agent.name),
                              ].join("\n")
                            : undefined
                        }
                      >
                        {usage
                          ? accessCountLabel(usage.access)
                          : insightStatusLabel(insights.status, "Access")}
                        {overrideCount > 0 &&
                          ` · ${overrideCount} ${overrideCount === 1 ? "override" : "overrides"}`}
                      </span>
                    </button>
                    <button
                      type="button"
                      onClick={() => toggle(key.id, "requests")}
                      aria-expanded={expanded && open.view === "requests"}
                      aria-controls={panelId}
                      aria-label={`Recent requests for ${key.label}`}
                      data-insight-view="requests"
                      data-connection-id={key.id}
                      title={
                        latest
                          ? `${recordedSourceLabel(latest, key)} · ${outcomeLabel(latest.outcome)} · ${latest.occurred_at}`
                          : useTracked
                            ? "No recorded use with exact connection attribution in the last 30 days"
                            : "Use is not reported by this server"
                      }
                      className="mt-1 flex w-full min-w-0 items-center gap-1.5 rounded-sm text-left text-11 hover:text-primary-text focus-visible:outline-2 focus-visible:outline-ring"
                    >
                      <Clock3
                        className="size-3.5 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                      <span className="shrink-0 whitespace-nowrap text-muted-foreground">
                        {usage?.activity.visibility === "own_requests"
                          ? "Your last use"
                          : "Last used"}{" "}
                        ·
                      </span>
                      {latest ? (
                        <span className="min-w-0 truncate">
                          {callerLabel(latest.caller)}
                          {latest.caller.app_name
                            ? ` · ${latest.caller.app_name}`
                            : ""}{" "}
                          · {formatRelativeTime(latest.occurred_at)} ·{" "}
                          {outcomeLabel(latest.outcome)}
                        </span>
                      ) : (
                        <span className="min-w-0 truncate">
                          {insights.status === "loading"
                            ? "Loading…"
                            : insights.status === "restricted"
                              ? "History restricted"
                              : insights.status === "error"
                                ? "History couldn't load"
                                : "Not recorded"}
                        </span>
                      )}
                    </button>
                  </TableCell>
                  <TableCell>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          onClick={() => toggle(key.id, "billing")}
                          aria-expanded={expanded && open.view === "billing"}
                          aria-controls={panelId}
                          aria-label={`Billing for ${key.label}`}
                          aria-description={
                            billingCategory === "not_billable"
                              ? "Not billable by NyxID"
                              : undefined
                          }
                          data-insight-view="billing"
                          data-connection-id={key.id}
                          className={cn(
                            "block max-w-full rounded-sm text-left text-xs hover:text-primary-text focus-visible:outline-2 focus-visible:outline-ring",
                            billingCategory === "not_billable"
                              ? "w-fit"
                              : "w-full",
                          )}
                        >
                          <span className="flex items-start gap-1.5 font-medium">
                            <CreditCard className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                            <span className="min-w-0 whitespace-normal break-words">
                              {insights.status === "ready"
                                ? connectionBillingLabels[billingCategory]
                                : insightStatusLabel(
                                    insights.status,
                                    "Billing",
                                  )}
                            </span>
                          </span>
                          {billing &&
                            (billingCategory === "platform" ||
                              billingCategory === "byok") && (
                              <span className="mt-1 block truncate text-11 text-muted-foreground">
                                {plainBilling(key, billing, catalog).short}
                              </span>
                            )}
                        </button>
                      </TooltipTrigger>
                      <TooltipContent
                        side={
                          billingCategory === "not_billable"
                            ? "right"
                            : "bottom"
                        }
                        align={
                          billingCategory === "not_billable"
                            ? "center"
                            : "start"
                        }
                        sideOffset={8}
                        collisionPadding={12}
                        className="max-w-xs whitespace-normal"
                      >
                        {billingCategory === "not_billable" ? (
                          <p>Not billable by NyxID</p>
                        ) : (
                          <>
                            <p className="font-medium">
                              {billing
                                ? plainBilling(key, billing, catalog).headline
                                : insightStatusLabel(
                                    insights.status,
                                    "Billing",
                                  )}
                            </p>
                            {billing && (
                              <p className="mt-1">
                                {plainBilling(key, billing, catalog).detail}
                              </p>
                            )}
                            <p className="mt-1 text-muted-foreground">
                              Click for details and your usage.
                            </p>
                          </>
                        )}
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                  <TableCell>
                    {editable ? (
                      <>
                        <p
                          className="truncate font-mono text-11"
                          title={target(key)}
                        >
                          {target(key)}
                        </p>
                        <p className="mt-1 flex flex-wrap items-center gap-x-2 text-11 text-muted-foreground">
                          <span className="truncate" title={configCounts}>
                            {authNames[key.auth_method] ?? key.auth_method} ·{" "}
                            {route}
                            {configCounts ? ` · ${configCounts}` : ""}
                          </span>
                          <Link
                            to="/keys/$keyId"
                            params={{ keyId: key.id }}
                            aria-label={`Configure ${key.label} (${owner})`}
                            className="inline-flex shrink-0 items-center gap-1 text-primary-text hover:underline"
                          >
                            <Settings2 className="size-3" aria-hidden="true" />{" "}
                            Configure
                          </Link>
                        </p>
                      </>
                    ) : (
                      <p className="flex items-center gap-1.5 text-11 text-muted-foreground">
                        <LockKeyhole
                          className="size-3 shrink-0"
                          aria-hidden="true"
                        />
                        {key.auto_connected
                          ? "Platform managed"
                          : "Editors only"}
                      </p>
                    )}
                    <p className="mt-1 flex flex-wrap items-center gap-x-2 text-11">
                      <span
                        className="truncate"
                        title={
                          changedAt
                            ? `${change ? `${activity} by ${change.actor.name}` : activity} · ${formatDateTime(changedAt)}`
                            : undefined
                        }
                      >
                        {activity}
                        {changedAt ? ` ${formatDate(changedAt)}` : ""}
                        {change ? ` · ${change.actor.name}` : ""}
                      </span>
                      <button
                        type="button"
                        onClick={() =>
                          onViewHistory
                            ? onViewHistory(key)
                            : toggle(key.id, "history")
                        }
                        aria-label={`History for ${key.label} (${owner})`}
                        aria-expanded={expanded && open.view === "history"}
                        aria-controls={panelId}
                        className="inline-flex shrink-0 items-center gap-1 rounded-sm text-primary-text hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                      >
                        <History className="size-3" aria-hidden="true" />{" "}
                        History
                      </button>
                    </p>
                  </TableCell>
                </TableRow>
                {memberships.length > 0 && (
                  <TableRow
                    data-service-connection-pools={key.id}
                    className={
                      expanded
                        ? "border-b-0 bg-muted/20 hover:bg-muted/20"
                        : "[tr:hover+&]:bg-overlay"
                    }
                  >
                    {/* Full width so pool names never stretch the connection column. */}
                    <TableCell
                      colSpan={5}
                      className="whitespace-normal pb-3 pt-0"
                    >
                      <p className="mb-1.5 text-11 font-medium text-muted-foreground">
                        Member of service pools
                      </p>
                      <ul
                        aria-label={`Pools using ${key.label}`}
                        className="flex flex-col items-start gap-1.5"
                      >
                        {memberships.map((pool) => {
                          const member = pool.members.find(
                            (member) => member.user_service_id === key.id,
                          )!;
                          return (
                            <li key={pool.id} className="min-w-0">
                              <button
                                type="button"
                                onClick={() => onViewPool?.(pool.id)}
                                className="flex min-w-0 items-start gap-1.5 rounded-sm text-left text-11 text-primary-text hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                                title={`${poolStrategyLabel(pool)} · ${pool.members.length} connections${!pool.is_active ? " · pool disabled" : ""}${!member.enabled ? " · member disabled" : ""}`}
                              >
                                <PoolStrategyIcon
                                  strategy={pool.strategy}
                                  className="mt-0.5 size-3 shrink-0"
                                />
                                <span className="min-w-0 break-words [overflow-wrap:anywhere]">
                                  {pool.name} ·{" "}
                                  {pool.strategy === "priority"
                                    ? `Priority ${member.priority ?? 0}`
                                    : poolStrategyLabel(pool)}
                                </span>
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    </TableCell>
                  </TableRow>
                )}
                <AnimatePresence initial={false}>
                  {expanded && (
                    <TableRow
                      key="panel"
                      id={panelId}
                      className="bg-muted/20 hover:bg-muted/20"
                    >
                      <TableCell colSpan={5} className="whitespace-normal p-0">
                        <motion.div {...reveal} className="overflow-clip">
                          <div className="px-3 pb-3">
                            <FadeIn
                              key={open.view}
                              className="rounded-xl border border-border/60 bg-card"
                            >
                              {open.view === "history" ? (
                                <PanelSection icon={History} title="History">
                                  <ServiceHistory serviceId={key.id} />
                                </PanelSection>
                              ) : open.view === "details" ? (
                                <PanelSection
                                  icon={Info}
                                  title="Connection details"
                                >
                                  <ConnectionMetadata
                                    connection={key}
                                    insight={insight}
                                  />
                                </PanelSection>
                              ) : (
                                <ConnectionInsightPanel
                                  key={`${key.id}:${open.view}`}
                                  connection={key}
                                  insight={insight}
                                  view={open.view}
                                  state={insights}
                                  catalog={catalog}
                                />
                              )}
                            </FadeIn>
                          </div>
                        </motion.div>
                      </TableCell>
                    </TableRow>
                  )}
                </AnimatePresence>
              </Fragment>
            );
          })}
        </TableBody>
      </Table>
    </TooltipProvider>
  );
}
