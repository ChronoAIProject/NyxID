import { useState } from "react";
import { GitBranch, Settings2 } from "lucide-react";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { usePoolHealth } from "@/hooks/use-pools";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  billingAccountLabel,
  billingModelLabel,
  credentialLabel,
  nyxidChargeLabel,
} from "@/lib/service-insights";
import {
  orderedPoolMembers,
  poolFailoverLabel,
  poolMemberStatus,
  poolStrategyLabel,
} from "@/lib/service-pool-display";
import { connectionSource, connectionSourceLabel } from "@/lib/service-view";
import { defaultFailoverPolicy, type ServicePool } from "@/schemas/pools";
import type { KeyInfo } from "@/types/keys";
import { ServiceOwnerAvatar } from "./service-owner-avatar";

export function ServicePoolRoutingPanel({
  pool,
  connections,
  insights,
  onEdit,
}: {
  readonly pool: ServicePool;
  readonly connections: readonly KeyInfo[];
  readonly insights: ServiceInsightsState;
  readonly onEdit?: () => void;
}) {
  const ai = pool.strategy === "priority" && pool.member_contract === "ai_chat";
  const [method, setMethod] = useState("POST");
  const [path, setPath] = useState("/");
  const [operation, setOperation] = useState({ method: "POST", path: "/" });
  const health = usePoolHealth({
    poolId: pool.id,
    contract: pool.member_contract,
    method: ai ? "POST" : operation.method,
    path: ai ? "chat/completions" : operation.path,
  });
  const inspected = new Map(
    (health.isError ? [] : (health.data?.candidates ?? [])).map((candidate) => [
      candidate.user_service_id,
      candidate,
    ]),
  );
  const keys = new Map(connections.map((key) => [key.id, key]));
  const members = orderedPoolMembers(pool);
  const policy = pool.failover ?? defaultFailoverPolicy;
  const priority = pool.strategy === "priority";

  return (
    <section
      aria-label={`${pool.name} routing`}
      className="min-w-0 space-y-3 p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <GitBranch className="size-4" aria-hidden="true" />
            <span>{pool.name}</span>
            <Badge variant="secondary">{poolStrategyLabel(pool)}</Badge>
            {!pool.is_active && <Badge variant="secondary">Disabled</Badge>}
          </div>
          <code className="block break-all text-xs text-primary">
            /api/v1/proxy/s/{pool.slug}
          </code>
          {ai && (
            <p className="text-xs text-muted-foreground">
              AI chat · Gateway model <code>pool:{pool.slug}</code>
            </p>
          )}
        </div>
        {onEdit && (
          <Button variant="outline" size="sm" onClick={onEdit}>
            <Settings2 className="size-3.5" />
            Configure pool
          </Button>
        )}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
        <span>{poolFailoverLabel(pool)}</span>
        {priority && (
          <>
            <span className="text-muted-foreground">
              {pool.tier_balance === "weighted" ? "Weighted" : "Round-robin"}{" "}
              within each priority
            </span>
            <span className="text-muted-foreground">
              {policy.per_attempt_timeout_ms / 1000}s per attempt ·{" "}
              {policy.overall_deadline_ms / 1000}s total
            </span>
          </>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {priority
          ? "Lower priority runs first; ineligible and cooling connections are skipped. Use this pool slug to apply its routing policy."
          : "Each request selects one eligible connection. Rotation applies when you call this pool slug."}
      </p>
      {ai ? (
        <p className="text-xs text-muted-foreground">
          Eligibility for POST chat/completions · refreshed every 15s
        </p>
      ) : (
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            setOperation({ method, path: path.trim() || "/" });
          }}
        >
          <label className="space-y-1 text-xs text-muted-foreground">
            <span>Method</span>
            <select
              aria-label={`Method for ${pool.name}`}
              value={method}
              onChange={(event) => setMethod(event.target.value)}
              className="block h-8 rounded-md border bg-background px-2 text-foreground"
            >
              {["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].map(
                (value) => (
                  <option key={value}>{value}</option>
                ),
              )}
            </select>
          </label>
          <label className="min-w-36 flex-1 space-y-1 text-xs text-muted-foreground">
            <span>Operation path</span>
            <Input
              aria-label={`Operation path for ${pool.name}`}
              value={path}
              onChange={(event) => setPath(event.target.value)}
              className="h-8"
            />
          </label>
          <Button type="submit" variant="outline" size="sm">
            Inspect
          </Button>
          <p className="basis-full text-[11px] text-muted-foreground">
            Showing {operation.method} {operation.path} · metadata only, no
            service call · refreshed every 15s
          </p>
        </form>
      )}
      {health.isError && (
        <p role="status" className="text-xs text-muted-foreground">
          Eligibility could not be loaded. Saved routing is shown; connection
          health is unverified.
        </p>
      )}
      <div className="overflow-hidden rounded-lg border">
        <Table
          aria-label={`${pool.name} route members`}
          className="min-w-[720px] table-fixed"
        >
          <TableHeader>
            <TableRow>
              <TableHead className="w-[14%]">
                {priority ? "Priority / Weight" : "Rotation"}
              </TableHead>
              <TableHead className="w-[25%]">Connection / Credential</TableHead>
              <TableHead className="w-[27%]">Billing / Payer</TableHead>
              <TableHead className="w-[34%]">Eligibility / Cooldown</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {members.map((member, index) => {
              const key = keys.get(member.user_service_id);
              const billing = insights.connections.get(
                member.user_service_id,
              )?.billing;
              const candidate = inspected.get(member.user_service_id);
              const org =
                key?.credential_source?.type === "org"
                  ? key.credential_source
                  : null;
              const status = poolMemberStatus(member, candidate);
              return (
                <TableRow
                  key={member.user_service_id}
                  data-service-connection-row={member.user_service_id}
                  className="[&>td]:align-top [&>td]:py-3"
                >
                  <TableCell className="text-xs">
                    <p>
                      {priority
                        ? `Priority ${member.priority ?? 0}`
                        : pool.strategy === "weighted"
                          ? `Weight ${member.weight}`
                          : `Member ${index + 1}`}
                    </p>
                    {priority && pool.tier_balance === "weighted" && (
                      <p className="mt-1 text-muted-foreground">
                        Weight {member.weight}
                      </p>
                    )}
                  </TableCell>
                  <TableCell className="space-y-1 text-xs">
                    <div className="flex items-center gap-1.5">
                      {key && (
                        <ServiceIcon
                          slug={key.catalog_service_slug ?? key.slug}
                          size="sm"
                        />
                      )}
                      <span
                        className="truncate"
                        title={key?.label ?? candidate?.slug}
                      >
                        {key?.label ??
                          candidate?.slug ??
                          "Unavailable connection"}
                      </span>
                    </div>
                    {(key?.slug || candidate?.slug) && (
                      <code className="block truncate text-[11px] text-muted-foreground">
                        {key?.slug ?? candidate?.slug}
                      </code>
                    )}
                    {key && (
                      <div className="flex items-center gap-1.5 text-muted-foreground">
                        <ServiceOwnerAvatar
                          type={connectionSource(key)}
                          name={connectionSourceLabel(key)}
                          avatarUrl={org?.avatar_url}
                          className="size-4"
                        />
                        <span>{connectionSourceLabel(key)}</span>
                      </div>
                    )}
                    <p className="text-[11px] text-muted-foreground">
                      {key
                        ? credentialLabel(key, billing)
                        : "Credential not reported"}
                    </p>
                    {ai && (
                      <p className="break-words text-[11px] text-muted-foreground">
                        Model: {member.model ?? "Not configured"}
                      </p>
                    )}
                  </TableCell>
                  <TableCell className="space-y-1 text-xs">
                    <p>{billingModelLabel(billing)}</p>
                    <p className="text-[11px] text-muted-foreground">
                      {billing?.context === "configuration"
                        ? "Expected payer"
                        : "Payer"}
                      : {billingAccountLabel(billing)}
                    </p>
                    <p className="text-[11px] text-muted-foreground">
                      {billing
                        ? nyxidChargeLabel(billing)
                        : "Rate not reported"}
                    </p>
                  </TableCell>
                  <TableCell className="space-y-1 text-xs">
                    <Badge
                      variant={
                        member.enabled && candidate?.eligible
                          ? "success"
                          : "secondary"
                      }
                    >
                      {health.isLoading && member.enabled
                        ? "Inspecting…"
                        : status}
                    </Badge>
                    {candidate && (
                      <p className="text-[11px] text-muted-foreground">
                        {candidate.consecutive_failures} failures
                        {candidate.last_status
                          ? ` · Last HTTP ${candidate.last_status}`
                          : ""}
                      </p>
                    )}
                    {candidate?.cooldown_until && (
                      <p className="text-[11px] text-muted-foreground">
                        Cooldown until{" "}
                        {new Date(candidate.cooldown_until).toLocaleString()}
                      </p>
                    )}
                    {!pool.is_active && (
                      <p className="text-[11px] text-muted-foreground">
                        Pool disabled · no execution
                      </p>
                    )}
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
        {!members.length && (
          <p className="p-4 text-xs text-muted-foreground">
            No connections in this pool.
          </p>
        )}
      </div>
      <p className="text-[11px] text-muted-foreground">
        Each attempted connection uses its own rates and billing account.
        Reported usage can charge more than one attempt. Within that account:
        eligible allowance → credit grants → wallet credits. Platform-key usage
        bills the acting person.
      </p>
      {priority && (
        <details className="text-xs text-muted-foreground">
          <summary className="w-fit cursor-pointer">
            Failover conditions
          </summary>
          <div className="mt-2 space-y-1">
            <p>
              Retry causes:{" "}
              {policy.retry_on.length
                ? policy.retry_on
                    .map((trigger) => trigger.replaceAll("_", " "))
                    .join(", ")
                : "None"}
              .
            </p>
            <p>
              Cooldown: {policy.cooldown.base_ms / 1000}–
              {policy.cooldown.max_ms / 1000}s after{" "}
              {policy.cooldown.failures_to_open} failure(s)
              {policy.cooldown.honor_retry_after
                ? "; honors provider Retry-After"
                : ""}
              .
            </p>
            <p>
              {policy.retry_ambiguous_dispatch
                ? "Ambiguous replay enabled: a timeout or server error can retry completed work and incur extra charges."
                : "Ambiguous replay off: a POST timeout or server error does not automatically retry work that may already have run."}{" "}
              Failover stops once response data reaches the caller. NyxID
              access, approval and billing errors stop the request.
            </p>
          </div>
        </details>
      )}
    </section>
  );
}
