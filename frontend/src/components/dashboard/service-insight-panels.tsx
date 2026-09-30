import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  Bot,
  CreditCard,
  UsersRound,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { metricLabel } from "@/schemas/billing-metrics";
import type { ServiceInsight } from "@/schemas/service-insights";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  accessReasonLabel,
  billingAccountLabel,
  billingModelLabel,
  billingExplanation,
  callerKindLabel,
  callerLabel,
  credentialLabel,
  outcomeLabel,
  recordedSourceLabel,
} from "@/lib/service-insights";
import { formatDateTime } from "@/lib/utils";
import type { KeyInfo } from "@/types/keys";

export type InsightPanel = "access" | "requests" | "billing";

export function InsightsUnavailable({
  state,
}: {
  readonly state: ServiceInsightsState;
}) {
  return (
    <div
      className="flex items-center justify-between gap-4 p-3 text-xs text-muted-foreground"
      role="status"
    >
      <p>
        {state.status === "loading"
          ? "Loading billing and caller information…"
          : state.status === "restricted"
            ? "You do not have permission to view these connection insights."
            : state.status === "unavailable"
              ? "This server does not provide connection billing and caller insights yet."
              : "Connection insights could not be loaded."}
      </p>
      {state.status !== "loading" && (
        <Button size="sm" variant="outline" onClick={state.refresh}>
          Retry
        </Button>
      )}
    </div>
  );
}

function ConnectionBillingPanel({
  connection,
  insight,
  state,
}: {
  readonly connection: KeyInfo;
  readonly insight: ServiceInsight;
  readonly state: ServiceInsightsState;
}) {
  const [caller, setCaller] = useState("you");
  const selectedState = useServiceInsights(
    [connection],
    caller === "you" ? state : undefined,
    caller === "you" ? undefined : caller,
  );
  const bill =
    caller === "you"
      ? insight.billing
      : selectedState.connections.get(connection.id)?.billing;
  return (
    <section
      className="space-y-4 p-3"
      aria-label={`Billing for ${connection.label}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="inline-flex items-center gap-2 text-sm font-medium">
          <CreditCard className="size-4 text-primary" /> Billing flow
        </h4>
        <div className="flex items-center gap-2 text-xs">
          {insight.billing?.context === "configuration" ? (
            <Badge variant="secondary">Connection default · configured</Badge>
          ) : (
            <>
              <span className="text-muted-foreground">For</span>
              <Select value={caller} onValueChange={setCaller}>
                <SelectTrigger
                  aria-label="Preview billing for"
                  className="h-8 w-52"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="you">You · connection default</SelectItem>
                  {insight.usage?.access.keys.map((key) => (
                    <SelectItem key={key.id} value={key.id}>
                      {key.name} · agent key
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </>
          )}
        </div>
      </div>
      {bill ? (
        <>
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <Badge variant="secondary">{billingModelLabel(bill)}</Badge>
            {bill.status !== "restricted" && bill.status !== "unavailable" && (
              <span className="text-muted-foreground">
                {billingExplanation(bill)}
              </span>
            )}
          </div>
          <dl className="grid items-center gap-3 text-xs sm:grid-cols-[1fr_auto_1fr_auto_1fr]">
            <div className="self-stretch rounded-lg border border-border/60 bg-muted/20 p-3">
              <dt className="text-muted-foreground">Credential</dt>
              <dd className="mt-1 font-medium">
                {credentialLabel(connection, bill)}
              </dd>
            </div>
            <ArrowRight
              className="hidden size-4 text-muted-foreground sm:block"
              aria-hidden="true"
            />
            <div className="self-stretch rounded-lg border border-border/60 bg-muted/20 p-3">
              <dt className="text-muted-foreground">
                {bill.context === "configuration"
                  ? "Expected payer"
                  : "Billing account"}
              </dt>
              <dd className="mt-1 font-medium">{billingAccountLabel(bill)}</dd>
            </div>
            <ArrowRight
              className="hidden size-4 text-muted-foreground sm:block"
              aria-hidden="true"
            />
            <div className="self-stretch rounded-lg border border-border/60 bg-muted/20 p-3">
              <dt className="text-muted-foreground">NyxID charges</dt>
              <dd className="mt-1 font-medium">
                {bill.charge_status === "not_charged"
                  ? "Not charged by NyxID"
                  : bill.charge_status === "usage_based"
                    ? "Based on metered usage"
                    : bill.charge_status === "restricted"
                      ? "Restricted"
                      : "Determined at execution"}
              </dd>
            </div>
          </dl>
          {bill.status !== "restricted" &&
            bill.status !== "unavailable" &&
            (bill.charge_status === "usage_based" ||
              bill.charge_status === "conditional") && (
              <div className="text-xs">
                <p>
                  <span className="text-muted-foreground">Funding order: </span>
                  Eligible allowances → Credit grants → Wallet credits
                </p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  Applied within the selected billing account, including
                  eligible platform-issued grants. Actual funding is determined
                  per request; the grant used is not reported in this preview.
                </p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  If a request fails, NyxID does not retry the other connections
                  in this service.
                </p>
              </div>
            )}
          {!!bill.rates.length && (
            <div className="overflow-x-auto rounded-lg border border-border/60">
              <table
                className="w-full text-left text-xs"
                aria-label={
                  bill.context === "configuration"
                    ? "Configured NyxID rates"
                    : "Applicable NyxID rates"
                }
              >
                <thead className="bg-muted/30 text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2 font-medium">Charge</th>
                    <th className="px-3 py-2 font-medium">Unit</th>
                    <th className="px-3 py-2 text-right font-medium">
                      Credits per unit
                    </th>
                    {bill.context === "configuration" && (
                      <th className="px-3 py-2 font-medium">Price sync</th>
                    )}
                  </tr>
                </thead>
                <tbody>
                  {bill.rates.map((rate, i) => (
                    <tr
                      key={`${rate.layer}-${rate.metric}-${i}`}
                      className="border-t border-border/40"
                    >
                      <td className="px-3 py-2">
                        {rate.layer === "resale"
                          ? "Provider usage through NyxID"
                          : "NyxID usage"}
                      </td>
                      <td className="px-3 py-2">
                        {metricLabel(rate.metric, 1)}
                      </td>
                      <td className="px-3 py-2 text-right font-mono">
                        {rate.credits_per_unit ?? "Plan rate not reported"}
                      </td>
                      {bill.context === "configuration" && (
                        <td className="px-3 py-2">
                          {rate.sync_status === "synced"
                            ? "Synced"
                            : rate.sync_status === "pending"
                              ? "Pending"
                              : rate.sync_status === "failed"
                                ? "Failed"
                                : "Not reported"}
                        </td>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {bill.notes.map((note) => (
            <p key={note} className="text-xs text-muted-foreground">
              {note}
            </p>
          ))}
        </>
      ) : (
        <InsightsUnavailable state={selectedState} />
      )}
    </section>
  );
}

export function ConnectionInsightPanel({
  connection,
  insight,
  view,
  state,
}: {
  readonly connection: KeyInfo;
  readonly insight?: ServiceInsight;
  readonly view: InsightPanel;
  readonly state: ServiceInsightsState;
}) {
  const [showAllKeys, setShowAllKeys] = useState(false);
  if (!insight || (view === "billing" ? !insight.billing : !insight.usage))
    return <InsightsUnavailable state={state} />;
  const usage = insight.usage;
  if (view === "billing")
    return (
      <ConnectionBillingPanel
        connection={connection}
        insight={insight}
        state={state}
      />
    );
  if (!usage) return <InsightsUnavailable state={state} />;
  if (view === "access")
    return (
      <section
        className="space-y-3 p-3"
        aria-label={`Agent key access for ${connection.label}`}
      >
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h4 className="inline-flex items-center gap-2 text-sm font-medium">
            <UsersRound className="size-4 text-primary" />{" "}
            {usage.access.basis === "configuration"
              ? "Agent keys in scope"
              : "Agent keys with access"}
          </h4>
          <span className="text-[11px] text-muted-foreground">
            {usage.access.visibility === "own_keys"
              ? "Your keys only"
              : "Keys you manage"}{" "}
            · current scope
          </span>
        </div>
        {usage.access.keys.length ? (
          <div className="overflow-x-auto">
            <table
              className="w-full text-left text-xs"
              aria-label="Agent keys with access"
            >
              <thead className="text-muted-foreground">
                <tr>
                  <th className="py-2 pr-3 font-medium">Agent key</th>
                  <th className="px-3 py-2 font-medium">Access through</th>
                  <th className="px-3 py-2 font-medium">Credential</th>
                </tr>
              </thead>
              <tbody>
                {(showAllKeys
                  ? usage.access.keys
                  : usage.access.keys.slice(0, 3)
                ).map((key) => (
                  <tr key={key.id} className="border-t border-border/40">
                    <td className="py-2.5 pr-3">
                      <Link
                        to="/keys/api-key/$keyId"
                        params={{ keyId: key.id }}
                        className="inline-flex items-center gap-1.5 font-medium text-primary hover:underline"
                      >
                        <Bot className="size-3.5" />
                        {key.name}
                        <ArrowUpRight className="size-3" />
                      </Link>
                      {key.platform && (
                        <span className="ml-2 text-muted-foreground">
                          {key.platform}
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-2.5">
                      {accessReasonLabel(key.permission)}
                    </td>
                    <td className="px-3 py-2.5">
                      {key.credential_override === null ? (
                        "Override not reported"
                      ) : key.credential_override ? (
                        <Badge variant="secondary">Credential override</Badge>
                      ) : (
                        "Connection default"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            {usage.access.visibility === "unavailable"
              ? "Agent key inventory could not be loaded."
              : usage.access.incomplete
                ? "No matching keys in the available inventory. Some key inventories could not be checked."
                : "No agent keys in your permitted inventory currently include this connection in their scope."}
          </p>
        )}
        {usage.access.keys.length > 3 && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setShowAllKeys(!showAllKeys)}
          >
            {showAllKeys
              ? "Show fewer keys"
              : `Show all ${usage.access.keys.length} keys`}
          </Button>
        )}
        {usage.access.truncated && (
          <p className="text-xs text-muted-foreground">
            Showing a limited set of keys. Open Agent keys to review the full
            inventory.
          </p>
        )}
        <p className="text-[11px] text-muted-foreground">
          {usage.access.basis === "configuration"
            ? "Configured scope; live permissions and credentials are checked at execution. "
            : ""}
          {usage.access.incomplete && usage.access.keys.length > 0
            ? "Some key inventories could not be checked. "
            : ""}
          Scope access does not prove a working connection or previous use. Open
          a key to manage its service scope or credential override.
        </p>
      </section>
    );
  if (usage.activity.tracking === "unavailable")
    return (
      <section
        className="space-y-2 p-3 text-xs"
        aria-label={`Recent requests for ${connection.label}`}
      >
        <h4 className="font-medium">Request attribution unavailable</h4>
        <p className="text-muted-foreground">
          This server does not yet report which agent key or application used
          this exact connection. Configured key access is shown separately; it
          is not evidence of use.
        </p>
      </section>
    );
  return (
    <section
      className="space-y-3 p-3"
      aria-label={`Recent requests for ${connection.label}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-sm font-medium">Recent requests</h4>
        <span className="text-[11px] text-muted-foreground">
          {usage.activity.visibility === "own_requests"
            ? "Your requests"
            : "Visible requests"}{" "}
          · {usage.activity.period_days} days
        </span>
      </div>
      {usage.activity.requests.length ? (
        <div className="overflow-x-auto">
          <table
            className="w-full text-left text-xs"
            aria-label="Recent connection requests"
          >
            <thead className="text-muted-foreground">
              <tr>
                <th className="py-2 pr-3 font-medium">Caller</th>
                <th className="px-3 py-2 font-medium">Type / application</th>
                <th className="px-3 py-2 font-medium">Recorded layer</th>
                <th className="px-3 py-2 font-medium">Time</th>
                <th className="px-3 py-2 font-medium">Outcome</th>
              </tr>
            </thead>
            <tbody>
              {usage.activity.requests.map((request) => (
                <tr key={request.id} className="border-t border-border/40">
                  <td className="py-2.5 pr-3 font-medium">
                    {callerLabel(request.caller)}
                  </td>
                  <td className="px-3 py-2.5">
                    {callerKindLabel(request.caller.kind)}
                    {request.caller.kind !== "session" && (
                      <span className="mt-0.5 block text-muted-foreground">
                        {request.caller.app_name ??
                          (request.caller.app_id
                            ? "Application name not recorded"
                            : "Application not recorded")}
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-2.5">
                    {recordedSourceLabel(request, connection)}
                  </td>
                  <td className="px-3 py-2.5">
                    <time dateTime={request.occurred_at}>
                      {formatDateTime(request.occurred_at)}
                    </time>
                  </td>
                  <td className="px-3 py-2.5">
                    {outcomeLabel(request.outcome)}
                    {request.response_status != null && (
                      <span className="ml-1 text-muted-foreground">
                        · {request.response_status}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          No requests with exact connection attribution were recorded in this
          period.
        </p>
      )}
      {usage.activity.request_count > 0 && (
        <p className="text-[11px] text-muted-foreground">
          {usage.activity.request_count.toLocaleString()} recorded
          {usage.activity.request_count === 1 ? " request" : " requests"}
          {usage.activity.truncated
            ? ` · showing the latest ${usage.activity.requests.length}`
            : " in this period"}
        </p>
      )}
      <p className="text-[11px] text-muted-foreground">
        {usage.activity.tracking === "partial"
          ? "Tracking is partial. Older requests and unsupported request paths may not identify this connection. "
          : ""}
        A shared agent key identifies the key, not every application using it. A
        received response does not confirm stream completion or a settled
        charge.
      </p>
    </section>
  );
}
