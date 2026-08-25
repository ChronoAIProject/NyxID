import { AlertTriangle, Pencil } from "lucide-react";
import type {
  CreditExpiryPolicy,
  CreditSchedule,
} from "@/schemas/billing-credits";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
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
    <div className="overflow-x-auto rounded-lg border border-border">
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
                  <div className="flex items-center justify-end gap-3">
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      title="Edit schedule"
                      onClick={() => onEdit(schedule)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Switch
                      checked={schedule.is_active}
                      disabled={updatePending}
                      aria-label={
                        schedule.is_active
                          ? "Pause schedule"
                          : "Resume schedule"
                      }
                      onCheckedChange={() => onToggle(schedule)}
                    />
                  </div>
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
                No credit schedules.
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </div>
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
    return `Disbursing ${formatNumber(period.disbursed_count)}`;
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
