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
  const pageCount = Math.max(1, Math.ceil(total / perPage));
  return (
    <div className="space-y-3">
      <div className="overflow-x-auto rounded-lg border border-border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Recipient</TableHead>
              <TableHead>Remaining</TableHead>
              <TableHead>Scope</TableHead>
              <TableHead>Expiry</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Reason</TableHead>
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
                  {!grant.recipient_billing_enabled ? <RolloutWarning /> : null}
                </TableCell>
                <TableCell>
                  {formatCredits(grant.remaining_micros)}{" "}
                  <span className="text-[11px] text-muted-foreground">
                    of {formatCredits(grant.amount_micros)}
                  </span>
                </TableCell>
                <TableCell>
                  {scopeLabel(
                    grant.scope.all_services,
                    grant.scope.service_slugs,
                  )}
                </TableCell>
                <TableCell>{formatDateTime(grant.expires_at)}</TableCell>
                <TableCell>
                  <GrantStatus grant={grant} />
                </TableCell>
                <TableCell className="max-w-56 truncate text-muted-foreground">
                  {grant.reason || "-"}
                </TableCell>
                {canWrite ? (
                  <TableCell className="text-right">
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      title="Revoke grant"
                      disabled={
                        grant.status !== "active" ||
                        grant.reserved_micros > 0 ||
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

function GrantStatus({ grant }: { readonly grant: AdminCreditGrant }) {
  if (grant.activation_state === "pending_activation") {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help" tabIndex={0}>
            <Badge variant="warning">Pending activation</Badge>
          </span>
        </TooltipTrigger>
        <TooltipContent>
          Waiting for billing-ledger confirmation. The grant is not spendable or
          visible to the recipient yet.
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
  return allServices
    ? "All services"
    : slugs.length <= 2
      ? slugs.join(", ")
      : `${slugs.slice(0, 2).join(", ")} +${String(slugs.length - 2)}`;
}

function formatCredits(micros: number) {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 }).format(micros / 1_000_000)} credits`;
}

function formatDateTime(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Never";
}
