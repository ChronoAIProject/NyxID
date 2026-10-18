import { AlertTriangle, MoreHorizontal, Pencil } from "lucide-react";
import type {
  CreditExpiryPolicy,
  CreditSchedule,
} from "@/schemas/billing-credits";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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

export function SchedulesTable({
  schedules,
  canWrite,
  updatePending,
  onEdit,
  onToggle,
}: {
  readonly schedules: readonly CreditSchedule[];
  readonly canWrite: boolean;
  readonly updatePending: boolean;
  readonly onEdit: (schedule: CreditSchedule) => void;
  readonly onToggle: (schedule: CreditSchedule) => void;
}) {
  return (
    <>
      <div className="hidden overflow-x-auto rounded-lg border border-border sm:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Amount</TableHead>
              <TableHead>Recurrence</TableHead>
              <TableHead>Expiry</TableHead>
              <TableHead>Targets</TableHead>
              <TableHead>Current period</TableHead>
              <TableHead>Status</TableHead>
              {canWrite ? (
                <TableHead className="text-right">Actions</TableHead>
              ) : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {schedules.map((schedule) => (
              <TableRow key={schedule.id}>
                <TableCell className="font-medium">
                  {formatNumber(schedule.amount_credits)} credits
                </TableCell>
                <TableCell className="capitalize">
                  {schedule.recurrence}
                </TableCell>
                <TableCell>{expiryLabel(schedule.expiry)}</TableCell>
                <TableCell>
                  <div>
                    {schedule.target_kind === "all_users"
                      ? "All owners"
                      : `${String(schedule.target_user_ids.length)} selected`}
                  </div>
                  <RolloutSummary schedule={schedule} />
                </TableCell>
                <TableCell>
                  <div>{periodLabel(schedule)}</div>
                  {schedule.skipped_periods > 0 ? (
                    <Badge variant="warning" className="mt-1">
                      {formatNumber(schedule.skipped_periods)} skipped period
                      {schedule.skipped_periods === 1 ? "" : "s"}
                    </Badge>
                  ) : null}
                </TableCell>
                <TableCell>
                  <Badge variant={schedule.is_active ? "success" : "secondary"}>
                    {schedule.is_active ? "Active" : "Paused"}
                  </Badge>
                </TableCell>
                {canWrite ? (
                  <TableCell className="text-right">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          aria-label="Recurring credits actions"
                        >
                          <MoreHorizontal className="h-4 w-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => onEdit(schedule)}>
                          <Pencil /> Edit recurring credits
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          disabled={updatePending}
                          onSelect={() => onToggle(schedule)}
                        >
                          {schedule.is_active
                            ? "Pause recurring credits"
                            : "Resume recurring credits"}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
            {schedules.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={canWrite ? 7 : 6}
                  className="py-10 text-center text-muted-foreground"
                >
                  No recurring credits yet.
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </div>
      <div className="space-y-2 sm:hidden">
        {schedules.map((schedule) => (
          <article
            key={schedule.id}
            className="space-y-3 rounded-lg border border-border p-3"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-[12px] font-medium">
                  {formatNumber(schedule.amount_credits)} credits ·{" "}
                  {schedule.recurrence}
                </p>
                <p className="text-[11px] text-muted-foreground">
                  {expiryLabel(schedule.expiry)}
                </p>
              </div>
              <Badge variant={schedule.is_active ? "success" : "secondary"}>
                {schedule.is_active ? "Active" : "Paused"}
              </Badge>
            </div>
            <div className="space-y-1 text-[11px] text-muted-foreground">
              <p>
                {schedule.target_kind === "all_users"
                  ? "All owners"
                  : `${String(schedule.target_user_ids.length)} selected`}
              </p>
              <p>{periodLabel(schedule)}</p>
              {schedule.skipped_periods > 0 ? (
                <Badge variant="warning">{formatNumber(schedule.skipped_periods)} skipped period{schedule.skipped_periods === 1 ? "" : "s"}</Badge>
              ) : null}
              <RolloutSummary schedule={schedule} />
            </div>
            {canWrite ? (
              <div className="flex justify-end">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      aria-label="Recurring credits mobile actions"
                    >
                      Actions
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onSelect={() => onEdit(schedule)}>
                      <Pencil /> Edit recurring credits
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      disabled={updatePending}
                      onSelect={() => onToggle(schedule)}
                    >
                      {schedule.is_active
                        ? "Pause recurring credits"
                        : "Resume recurring credits"}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            ) : null}
          </article>
        ))}
        {schedules.length === 0 ? (
          <div className="rounded-lg border border-border py-10 text-center text-muted-foreground">
            No recurring credits yet.
          </div>
        ) : null}
      </div>
    </>
  );
}

function RolloutSummary({ schedule }: { readonly schedule: CreditSchedule }) {
  const disabled = schedule.recipients?.filter(
    (recipient) => !recipient.recipient_billing_enabled,
  ).length;
  if (!disabled) return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="mt-1 inline-flex cursor-help" tabIndex={0}>
          <Badge variant="warning" className="gap-1">
            <AlertTriangle className="h-3 w-3" />
            {disabled} rollout off
          </Badge>
        </span>
      </TooltipTrigger>
      <TooltipContent>
        Selected owners outside the billing rollout cannot see billing yet.
      </TooltipContent>
    </Tooltip>
  );
}

function expiryLabel(expiry: CreditExpiryPolicy) {
  switch (expiry.kind) {
    case "end_of_period":
      return "End of period";
    case "after_days":
      return `After ${String(expiry.days)} day${expiry.days === 1 ? "" : "s"}`;
    case "never":
      return "Never";
  }
}

function periodLabel(schedule: CreditSchedule) {
  const period = schedule.current_period;
  if (!period) return schedule.is_active ? "Pending" : "No open period";
  if (period.status === "disbursing") {
    return `Issuing ${formatNumber(period.disbursed_count)}`;
  }
  return `Complete - ${formatNumber(period.disbursed_count)} - expires ${
    period.expires_at ? formatDate(period.expires_at) : "never"
  }`;
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  }).format(new Date(value));
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}
