import { useEffect, useState } from "react";
import {
  decimalCredits,
  exactCredits,
  formatExactCredits,
  parseCredits,
} from "@/lib/credits";
import { hasCredits } from "@/lib/credits";
import { billingTargetLabel } from "@/lib/billing-targets";
import { AlertTriangle, Trash2 } from "lucide-react";
import type { AdminCreditGrant } from "@/schemas/billing-credits";
import { Badge } from "@/components/ui/badge";
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
  readonly allocationPolicies?: boolean;
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
  allocationPolicies = false,
  canWrite,
  revokePending,
  page,
  perPage,
  total,
  fetching,
  onPageChange,
  onRevoke,
}: CreditGrantsTableProps) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(timer);
  }, []);
  const pageCount = Math.max(1, Math.ceil(total / perPage));
  return (
    <div className="space-y-3">
      <div className="overflow-x-auto rounded-lg border border-border">
        <Table className="max-sm:block">
          <TableHeader className="max-sm:hidden">
            <TableRow>
              <TableHead>Recipient</TableHead>
              <TableHead>Allocated / used / remaining</TableHead>
              <TableHead>Scope</TableHead>
              <TableHead>Expiry</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Reason</TableHead>
              {canWrite ? (
                <TableHead className="text-right">Actions</TableHead>
              ) : null}
            </TableRow>
          </TableHeader>
          <TableBody className="max-sm:block">
            {grants.map((grant) => (
              <TableRow
                key={grant.id}
                className="max-sm:grid max-sm:gap-1 max-sm:p-3 max-sm:[&>td]:min-w-0 max-sm:[&>td]:whitespace-normal max-sm:[&>td]:break-words max-sm:[&>td]:border-0 max-sm:[&>td]:px-0"
              >
                <TableCell>
                  <div className="font-medium">
                    {grant.recipient_display_name ||
                      grant.recipient_email ||
                      grant.recipient_user_id}
                  </div>
                  {grant.recipient_display_name && grant.recipient_email ? (
                    <div className="text-11 text-muted-foreground">
                      {grant.recipient_email}
                    </div>
                  ) : null}
                  {grant.target_kind === "org_members" ||
                  grant.target_kind === "groups" ? (
                    <div className="text-11 text-muted-foreground">
                      {billingTargetLabel(grant)}
                    </div>
                  ) : null}
                  {!grant.recipient_billing_enabled ? <RolloutWarning /> : null}
                </TableCell>
                <TableCell>
                  <AllocationAmounts grant={grant} />
                </TableCell>
                <TableCell>
                  {scopeLabel(
                    grant.scope.all_services,
                    grant.scope.service_slugs,
                  )}
                  {grant.service_limits?.map((rule) => (
                    <div
                      key={rule.key}
                      className="mt-2 max-w-sm space-y-1 break-words text-11"
                    >
                      <p className="font-medium">
                        {rule.label || "Service limit"}:{" "}
                        {rule.service_slugs.join(", ")}
                      </p>
                      <p className="break-all">
                        Up to {formatExactCredits(rule.limit, 12)} credits ·
                        used {formatExactCredits(rule.consumed, 12)} · held{" "}
                        {formatExactCredits(rule.held ?? rule.reserved, 12)}
                      </p>
                    </div>
                  ))}
                </TableCell>
                <TableCell>{formatDateTime(grant.expires_at)}</TableCell>
                <TableCell>
                  <GrantStatus grant={grant} now={now} />
                </TableCell>
                <TableCell className="max-w-56 whitespace-normal break-words text-muted-foreground">
                  {grant.reason || "-"}
                </TableCell>
                {canWrite ? (
                  <TableCell className="text-right">
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      title="Reclaim remaining credits"
                      aria-label="Reclaim remaining credits"
                      disabled={
                        grant.status !== "active" ||
                        Boolean(grant.revoke_requested_at) ||
                        (!allocationPolicies &&
                          hasCredits(
                            grant.reserved ?? grant.reserved_micros,
                          )) ||
                        revokePending
                      }
                      onClick={() => onRevoke(grant)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
            {grants.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={canWrite ? 7 : 6}
                  className="py-10 text-center text-muted-foreground"
                >
                  No credit grants.
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </div>
      {pageCount > 1 ? (
        <div className="flex items-center justify-between text-12 text-muted-foreground">
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
  if (grant.status === "active" && grant.revoke_requested_at)
    return (
      <Badge variant="warning">Reclaim pending — held requests finishing</Badge>
    );
  if (
    grant.status === "active" &&
    grant.expires_at &&
    Date.parse(grant.expires_at) <= now
  )
    return <Badge variant="warning">Expired</Badge>;
  if (
    grant.status === "active" &&
    grant.activation_state === "pending_activation"
  ) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help" tabIndex={0}>
            <Badge variant="warning">Pending activation</Badge>
          </span>
        </TooltipTrigger>
        <TooltipContent>
          These issued credits are pending activation and cannot be used yet.
        </TooltipContent>
      </Tooltip>
    );
  }

  const variants = {
    active: "success",
    consumed: "secondary",
    expired: "warning",
    revoked: "destructive",
  } as const;
  return (
    <Badge variant={variants[grant.status]} className="capitalize">
      {grant.status}
    </Badge>
  );
}

function scopeLabel(allServices: boolean, slugs: readonly string[]) {
  return allServices ? "All services" : slugs.join(", ");
}

function formatDateTime(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Never";
}

function AllocationAmounts({ grant }: { readonly grant: AdminCreditGrant }) {
  const amount = exactCredits(grant.amount, grant.amount_micros) ?? "0";
  const remaining =
    exactCredits(grant.remaining, grant.remaining_micros) ?? "0";
  const held =
    grant.held ?? exactCredits(grant.reserved, grant.reserved_micros) ?? "0";
  const terminal = grant.terminal_amount ?? "0";
  const used =
    grant.terminal_amount === undefined &&
    (grant.status === "expired" || grant.status === "revoked")
      ? null
      : decimalCredits(
          parseCredits(amount) -
            parseCredits(remaining) -
            parseCredits(terminal),
        );
  return (
    <dl className="min-w-0 whitespace-normal space-y-1 text-12 tabular-nums">
      {[
        ["Allocated", amount],
        ["Used", used],
        ["Remaining", remaining],
        ["Held", held],
        ...(grant.reclaimed && parseCredits(grant.reclaimed) > 0n
          ? [["Reclaimed", grant.reclaimed]]
          : []),
      ].map(([label, value]) => (
        <div key={label} className="min-w-0">
          <dt className="inline text-muted-foreground">{label}: </dt>
          <dd className="inline break-all">
            {value === null
              ? "Unavailable from this server"
              : `${formatExactCredits(value ?? "0", 12)} credits`}
          </dd>
        </div>
      ))}
    </dl>
  );
}
