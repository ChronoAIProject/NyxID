import { useEffect, useState } from "react";
import { AlertTriangle } from "lucide-react";
import type { AdminCreditGrant } from "@/schemas/billing-credits";
import { Badge } from "@/components/ui/badge";
import { formatMicrocredits } from "@/lib/credits";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

interface CreditGrantsTableProps {
  readonly grants: readonly AdminCreditGrant[];
  readonly canWrite: boolean;
  readonly revokePending: boolean;
  readonly page: number;
  readonly perPage: number;
  readonly total: number;
  readonly fetching: boolean;
  readonly onPageChange: (page: number) => void;
  readonly onRevoke: (grant: AdminCreditGrant) => void;
}

export function CreditGrantsTable({
  grants,
  canWrite,
  revokePending,
  page,
  perPage,
  total,
  fetching,
  onPageChange,
  onRevoke,
}: CreditGrantsTableProps) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    // One clock for the entire page, scheduled at the nearest visible expiry.
    const current = Date.now();
    const untilNextExpiry = grants.reduce((next, grant) => {
      const expiry =
        grant.status === "active" && grant.expires_at
          ? new Date(grant.expires_at).getTime()
          : Infinity;
      return expiry > now
        ? Math.min(next, Math.max(0, expiry - current))
        : next;
    }, Infinity);
    if (!Number.isFinite(untilNextExpiry)) return;
    const timeout = window.setTimeout(
      () => setNow(Date.now()),
      Math.min(30_000, Math.max(1, untilNextExpiry)),
    );
    return () => window.clearTimeout(timeout);
  }, [grants, now]);
  const pageCount = Math.max(1, Math.ceil(total / perPage));
  return (
    <div className="space-y-3">
      <div className="hidden overflow-x-auto rounded-lg border border-border sm:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Recipient</TableHead>
              <TableHead>Allocated</TableHead>
              <TableHead>Used</TableHead>
              <TableHead>Remaining</TableHead>
              <TableHead>Services &amp; limits</TableHead>
              <TableHead>Expiry</TableHead>
              <TableHead>Status</TableHead>
              {canWrite ? (
                <TableHead className="text-right">Actions</TableHead>
              ) : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {grants.map((grant) => (
              <TableRow key={grant.id}>
                <TableCell>
                  <div className="font-medium">
                    {grant.recipient_display_name ||
                      grant.recipient_email ||
                      grant.recipient_user_id}
                  </div>
                  {grant.recipient_display_name && grant.recipient_email ? (
                    <div className="text-[11px] text-muted-foreground">
                      {grant.recipient_email}
                    </div>
                  ) : null}
                  {grant.reason ? (
                    <p className="mt-1 max-w-52 whitespace-normal text-[11px] text-muted-foreground">
                      {grant.reason}
                    </p>
                  ) : null}
                  {!grant.recipient_billing_enabled ? <RolloutWarning /> : null}
                </TableCell>
                <TableCell className="tabular-nums">
                  {formatMicrocredits(
                    grant.allocated_micros_exact ?? String(grant.amount_micros),
                  )}
                </TableCell>
                <TableCell className="tabular-nums">
                  {usedCredits(grant)}
                </TableCell>
                <TableCell className="font-medium tabular-nums">
                  {formatMicrocredits(
                    grant.remaining_micros_exact ??
                      String(grant.remaining_micros),
                  )}
                  {BigInt(
                    grant.reserved_micros_exact ??
                      String(grant.reserved_micros),
                  ) > 0n ? (
                    <div className="text-[11px] text-warning">
                      {formatMicrocredits(
                        grant.reserved_micros_exact ??
                          String(grant.reserved_micros),
                      )}{" "}
                      held; reclaim can be requested now
                    </div>
                  ) : null}
                  {BigInt(grant.reclaimed_micros ?? "0") > 0n ? (
                    <p className="mt-1 text-[11px] font-normal text-muted-foreground">
                      {formatMicrocredits(grant.reclaimed_micros!)} reclaimed
                    </p>
                  ) : null}
                </TableCell>
                <TableCell className="max-w-72 whitespace-normal break-words">
                  {scopeLabel(
                    grant.scope.all_services,
                    grant.scope.service_slugs,
                  )}
                  <ServiceLimitsSummary grant={grant} />
                </TableCell>
                <TableCell>{formatDateTime(grant.expires_at)}</TableCell>
                <TableCell>
                  <GrantStatus grant={grant} now={now} />
                </TableCell>
                {canWrite ? (
                  <TableCell className="text-right">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      title="Reclaim remaining credits"
                      aria-label="Reclaim remaining credits"
                      disabled={
                        grant.status !== "active" ||
                        Boolean(grant.revoke_requested_at) ||
                        revokePending
                      }
                      onClick={() => onRevoke(grant)}
                    >
                      Reclaim
                    </Button>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
            {grants.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={canWrite ? 8 : 7}
                  className="py-10 text-center text-muted-foreground"
                >
                  No issued credits.
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </div>
      <div className="space-y-2 sm:hidden">
        {grants.map((grant) => (
          <article
            key={grant.id}
            className="space-y-3 rounded-lg border border-border p-3"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="break-words text-[12px] font-medium">
                  {grant.recipient_display_name ||
                    grant.recipient_email ||
                    grant.recipient_user_id}
                </p>
                {grant.recipient_display_name && grant.recipient_email ? (
                  <p className="break-words text-[11px] text-muted-foreground">
                    {grant.recipient_email}
                  </p>
                ) : null}
              </div>
              <GrantStatus grant={grant} now={now} />
            </div>

            {!grant.recipient_billing_enabled ? <RolloutWarning /> : null}
            <div className="flex justify-between text-[12px] text-muted-foreground">
              <span>
                Allocated{" "}
                {formatMicrocredits(
                  grant.allocated_micros_exact ?? String(grant.amount_micros),
                )}
              </span>
              <span>Used {usedCredits(grant)}</span>
            </div>
            <div className="flex items-baseline justify-between gap-3 text-[12px]">
              <span className="text-muted-foreground">Remaining</span>
              <span className="text-right font-medium">
                {formatMicrocredits(
                  grant.remaining_micros_exact ??
                    String(grant.remaining_micros),
                )}{" "}
                of{" "}
                {formatMicrocredits(
                  grant.allocated_micros_exact ?? String(grant.amount_micros),
                )}
              </span>
            </div>
            {BigInt(
              grant.reserved_micros_exact ?? String(grant.reserved_micros),
            ) > 0n ? (
              <p className="text-[11px] text-warning">
                {formatMicrocredits(
                  grant.reserved_micros_exact ?? String(grant.reserved_micros),
                )}{" "}
                held; reclaim can be requested now
              </p>
            ) : null}
            {BigInt(grant.reclaimed_micros ?? "0") > 0n ? (
              <p className="text-[11px] text-muted-foreground">
                {formatMicrocredits(grant.reclaimed_micros!)} reclaimed
              </p>
            ) : null}
            <div className="space-y-1 text-[11px] text-muted-foreground">
              <p className="break-words">
                Scope:{" "}
                {scopeLabel(
                  grant.scope.all_services,
                  grant.scope.service_slugs,
                )}
              </p>
              <ServiceLimitsSummary grant={grant} />
              <p>Expiry: {formatDateTime(grant.expires_at)}</p>
              {grant.reason ? (
                <p className="break-words">Reason: {grant.reason}</p>
              ) : null}
            </div>
            {canWrite ? (
              <div className="flex justify-end">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={
                    grant.status !== "active" ||
                    Boolean(grant.revoke_requested_at) ||
                    revokePending
                  }
                  onClick={() => onRevoke(grant)}
                >
                  Reclaim remaining credits
                </Button>
              </div>
            ) : null}
          </article>
        ))}
        {grants.length === 0 ? (
          <div className="rounded-lg border border-border py-10 text-center text-muted-foreground">
            No issued credits.
          </div>
        ) : null}
      </div>
      {pageCount > 1 ? (
        <div className="flex items-center justify-between text-[12px] text-muted-foreground">
          <span>
            Page {page} of {pageCount}
          </span>
          <div className="flex gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={page <= 1 || fetching}
              onClick={() => onPageChange(Math.max(1, page - 1))}
            >
              Previous
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={page >= pageCount || fetching}
              onClick={() => onPageChange(Math.min(pageCount, page + 1))}
            >
              Next
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function RolloutWarning() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="mt-1 inline-flex cursor-help" tabIndex={0}>
          <Badge variant="warning" className="gap-1">
            <AlertTriangle className="h-3 w-3" />
            Billing rollout off
          </Badge>
        </span>
      </TooltipTrigger>
      <TooltipContent>
        Not in billing rollout - user cannot see billing yet.
      </TooltipContent>
    </Tooltip>
  );
}

function GrantStatus({
  grant,
  now,
}: {
  readonly grant: AdminCreditGrant;
  readonly now: number;
}) {
  const expiredAt = grant.expires_at
    ? new Date(grant.expires_at).getTime()
    : null;
  const wallClockExpired =
    grant.status === "active" && expiredAt !== null && expiredAt <= now;
  if (grant.status !== "active") {
    const variants = {
      active: "success",
      consumed: "secondary",
      expired: "warning",
      revoked: "destructive",
    } as const;
    return (
      <Badge variant={variants[grant.status]} className="capitalize">
        {grant.status === "revoked" ? "Reclaimed" : grant.status}
      </Badge>
    );
  }
  if (grant.revoke_requested_at)
    return <Badge variant="warning">Reclaim pending</Badge>;
  if (wallClockExpired) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help" tabIndex={0}>
            <Badge variant="warning">Expired</Badge>
          </span>
        </TooltipTrigger>
        <TooltipContent>
          Eligibility ended at the expiry time; expiry processing may still be
          pending.
        </TooltipContent>
      </Tooltip>
    );
  }
  if (grant.activation_state === "pending_activation") {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help" tabIndex={0}>
            <Badge variant="warning">Pending activation</Badge>
          </span>
        </TooltipTrigger>
        <TooltipContent>
          Waiting for activation confirmation. The credits are not spendable or
          visible to the recipient yet.
        </TooltipContent>
      </Tooltip>
    );
  }

  return <Badge variant="success">Active</Badge>;
}

function scopeLabel(allServices: boolean, slugs: readonly string[]) {
  return allServices ? "All services" : slugs.join(", ");
}

function usedCredits(grant: AdminCreditGrant) {
  if (grant.used_micros !== undefined)
    return formatMicrocredits(grant.used_micros);
  if (grant.status === "revoked" || grant.status === "expired") return "—";
  return formatMicrocredits(
    String(Math.max(0, grant.amount_micros - grant.remaining_micros)),
  );
}

function ServiceLimitsSummary({ grant }: { readonly grant: AdminCreditGrant }) {
  return (
    <div className="mt-1 space-y-1 text-[11px] text-muted-foreground">
      {grant.service_limits?.map((rule) => (
        <p key={rule.key}>
          {rule.label || rule.service_slugs.join(", ")}
          {rule.label &&
          rule.label.toLowerCase() !==
            rule.service_slugs.join(", ").toLowerCase()
            ? ` (${rule.service_slugs.join(", ")})`
            : ""}{" "}
          · up to {formatMicrocredits(rule.limit_micros)}
        </p>
      ))}
    </div>
  );
}

function formatDateTime(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Never";
}
