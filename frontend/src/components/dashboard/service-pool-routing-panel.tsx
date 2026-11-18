import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { Label } from "@/components/ui/label";
import { useState } from "react";
import { Settings2 } from "lucide-react";
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
import { PoolStrategyIcon, ServicePoolIcon } from "./service-pool-icons";
import { connectionIconUrl } from "@/lib/connection-icon";

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
  useUiTranslation();
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
      aria-label={translateUi("{{value1}} routing", { value1: pool.name })}
      className="min-w-0 space-y-3 p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2 type-row-title">
            <ServicePoolIcon className="size-4 shrink-0" />
            <span>{pool.name}</span>
            <Badge variant="secondary" className="gap-1.5">
              <PoolStrategyIcon
                strategy={pool.strategy}
                className="size-3 shrink-0"
              />
              {poolStrategyLabel(pool)}
            </Badge>
            {!pool.is_active && <Badge variant="secondary">{translateUi("Disabled")}</Badge>}
          </div>
          <code className="block break-all text-14 text-primary-text">
            /api/v1/proxy/s/{pool.slug}
          </code>
          {ai && (
            <p className="text-13 text-muted-foreground">{translateUi("AI chat · Gateway model ")}<code>pool:{pool.slug}</code>
            </p>
          )}
        </div>
        {onEdit && (
          <Button variant="outline" size="sm" onClick={onEdit}>
            <Settings2 className="size-3.5" />{translateUi("Configure pool")}</Button>
        )}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-13">
        <span>{poolFailoverLabel(pool)}</span>
        {priority && (
          <>
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <PoolStrategyIcon
                strategy={pool.tier_balance ?? "round_robin"}
                className="size-3 shrink-0"
              />
              {pool.tier_balance === "weighted" ? translateUi("Weighted") : translateUi("Round-robin")}{" "}{translateUi("within each priority")}</span>
            <span className="text-muted-foreground">
              {policy.per_attempt_timeout_ms / 1000}{translateUi("s per attempt ·")}{" "}
              {policy.overall_deadline_ms / 1000}{translateUi("s total")}</span>
          </>
        )}
      </div>
      <p className="text-13 text-muted-foreground">
        {priority
          ? translateUi("Lower priority runs first; ineligible and cooling connections are skipped. Use this pool slug to apply its routing policy.")
          : translateUi("Each request selects one eligible connection. Rotation applies when you call this pool slug.")}
      </p>
      {ai ? (
        <p className="text-13 text-muted-foreground">{translateUi("Eligibility for POST chat/completions · refreshed every 15s")}</p>
      ) : (
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            setOperation({ method, path: path.trim() || "/" });
          }}
        >
          <Label className="space-y-1 text-muted-foreground">
            <span>{translateUi("Method")}</span>
            <select
              aria-label={translateUi("Method for {{value1}}", { value1: pool.name })}
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
          </Label>
          <Label className="min-w-36 flex-1 space-y-1 text-muted-foreground">
            <span>{translateUi("Operation path")}</span>
            <Input
              aria-label={translateUi("Operation path for {{value1}}", { value1: pool.name })}
              value={path}
              onChange={(event) => setPath(event.target.value)}
              className="h-8"
            />
          </Label>
          <Button type="submit" variant="outline" size="sm">{translateUi("Inspect")}</Button>
          <p className="basis-full text-13 text-muted-foreground">{translateUi("Showing ")}{operation.method} {operation.path}{translateUi(" · metadata only, no service call · refreshed every 15s")}</p>
        </form>
      )}
      {health.isError && (
        <p role="status" className="text-13 text-muted-foreground">{translateUi("Eligibility could not be loaded. Saved routing is shown; connection health is unverified.")}</p>
      )}
      <div className="overflow-hidden rounded-lg border">
        <Table
          aria-label={translateUi("{{value1}} route members", { value1: pool.name })}
          className="min-w-[720px] table-fixed"
        >
          <TableHeader>
            <TableRow>
              <TableHead className="w-[14%]">
                {priority ? translateUi("Priority / Weight") : translateUi("Rotation")}
              </TableHead>
              <TableHead className="w-[25%]">{translateUi("Connection / Credential")}</TableHead>
              <TableHead className="w-[27%]">{translateUi("Billing / Payer")}</TableHead>
              <TableHead className="w-[34%]">{translateUi("Eligibility / Cooldown")}</TableHead>
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
                  <TableCell className="text-14">
                    <p>
                      {priority
                        ? translateUi("Priority {{value1}}", { value1: member.priority ?? 0 })
                        : pool.strategy === "weighted"
                          ? translateUi("Weight {{value1}}", { value1: member.weight })
                          : translateUi("Member {{value1}}", { value1: index + 1 })}
                    </p>
                    {priority && pool.tier_balance === "weighted" && (
                      <p className="mt-1 text-muted-foreground">{translateUi("Weight ")}{member.weight}
                      </p>
                    )}
                  </TableCell>
                  <TableCell className="space-y-1 text-14">
                    <div className="flex items-center gap-1.5">
                      {key && (
                        <ServiceIcon
                          slug={key.catalog_service_slug ?? key.slug}
                          iconUrl={connectionIconUrl(key)}
                          size="sm"
                        />
                      )}
                      <span
                        className="truncate"
                        title={key?.label ?? candidate?.slug}
                      >
                        {key?.label ??
                          candidate?.slug ??
                          translateUi("Unavailable connection")}
                      </span>
                    </div>
                    {(key?.slug || candidate?.slug) && (
                      <code className="block truncate text-14 text-muted-foreground">
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
                    <p className="text-13 text-muted-foreground">
                      {key
                        ? credentialLabel(key, billing)
                        : translateUi("Credential not reported")}
                    </p>
                    {ai && (
                      <p className="break-words text-13 text-muted-foreground">{translateUi("Model: ")}{member.model ?? translateUi("Not configured")}
                      </p>
                    )}
                  </TableCell>
                  <TableCell className="space-y-1 text-14">
                    <p>{billingModelLabel(billing)}</p>
                    <p className="text-13 text-muted-foreground">
                      {billing?.context === "configuration"
                        ? translateUi("Expected payer")
                        : translateUi("Payer")}
                      : {billingAccountLabel(billing)}
                    </p>
                    <p className="text-13 text-muted-foreground">
                      {billing
                        ? nyxidChargeLabel(billing)
                        : translateUi("Rate not reported")}
                    </p>
                  </TableCell>
                  <TableCell className="space-y-1 text-14">
                    <Badge
                      variant={
                        member.enabled && candidate?.eligible
                          ? "success"
                          : "secondary"
                      }
                    >
                      {health.isLoading && member.enabled
                        ? translateUi("Inspecting…")
                        : status}
                    </Badge>
                    {candidate && (
                      <p className="text-13 text-muted-foreground">
                        {candidate.consecutive_failures}{translateUi(" failures")}{candidate.last_status
                          ? translateUi(" · Last HTTP {{value1}}", { value1: candidate.last_status })
                          : ""}
                      </p>
                    )}
                    {candidate?.cooldown_until && (
                      <p className="text-13 text-muted-foreground">{translateUi("Cooldown until")}{" "}
                        {new Date(candidate.cooldown_until).toLocaleString(getFormattingLocale())}
                      </p>
                    )}
                    {!pool.is_active && (
                      <p className="text-13 text-muted-foreground">{translateUi("Pool disabled · no execution")}</p>
                    )}
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
        {!members.length && (
          <p className="p-4 text-13 text-muted-foreground">{translateUi("No connections in this pool.")}</p>
        )}
      </div>
      <p className="text-13 text-muted-foreground">{translateUi("Each attempted connection uses its own rates and billing account. Reported usage can charge more than one attempt. Within that account: eligible allowance → credit grants → wallet credits. Platform-key usage bills the acting person.")}</p>
      {priority && (
        <details className="text-13 text-muted-foreground">
          <summary className="w-fit cursor-pointer">{translateUi("Failover conditions")}</summary>
          <div className="mt-2 space-y-1">
            <p>{translateUi("Retry causes:")}{" "}
              {policy.retry_on.length
                ? policy.retry_on
                    .map((trigger) => trigger.replaceAll("_", " "))
                    .join(", ")
                : translateUi("None")}
              .
            </p>
            <p>{translateUi("Cooldown: ")}{policy.cooldown.base_ms / 1000}–
              {policy.cooldown.max_ms / 1000}{translateUi("s after")}{" "}
              {policy.cooldown.failures_to_open}{translateUi(" failure(s)")}{policy.cooldown.honor_retry_after
                ? translateUi("; honors provider Retry-After")
                : ""}
              .
            </p>
            <p>
              {policy.retry_ambiguous_dispatch
                ? translateUi("Ambiguous replay enabled: a timeout or server error can retry completed work and incur extra charges.")
                : translateUi("Ambiguous replay off: a POST timeout or server error does not automatically retry work that may already have run.")}{" "}{translateUi("Failover stops once response data reaches the caller. NyxID access, approval and billing errors stop the request.")}</p>
          </div>
        </details>
      )}
    </section>
  );
}
