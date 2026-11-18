import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { exactCredits, formatExactCredits } from "@/lib/credits";
import { billingTargetLabel } from "@/lib/billing-targets";
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
  useUiTranslation();
  return (
    <div className="overflow-x-auto rounded-lg border border-border">
      <Table className="max-sm:block">
        <TableHeader className="max-sm:hidden">
          <TableRow>
            <TableHead>{translateUi("Amount")}</TableHead>
            <TableHead>{translateUi("Recurrence")}</TableHead>
            <TableHead>{translateUi("Expiry")}</TableHead>
            <TableHead>{translateUi("Targets")}</TableHead>
            <TableHead>{translateUi("Current period")}</TableHead>
            <TableHead>{translateUi("Status")}</TableHead>
            {canWrite ? (
              <TableHead className="text-right">{translateUi("Actions")}</TableHead>
            ) : null}
          </TableRow>
        </TableHeader>
        <TableBody className="max-sm:block">
          {schedules.map((schedule) => (
            <TableRow
              key={schedule.id}
              className="max-sm:grid max-sm:gap-1 max-sm:p-3 max-sm:[&>td]:min-w-0 max-sm:[&>td]:whitespace-normal max-sm:[&>td]:break-words max-sm:[&>td]:px-0"
            >
              <TableCell className="font-normal">
                {formatExactCredits(
                  exactCredits(
                    schedule.amount,
                    schedule.amount_credits,
                    "whole",
                  ) ?? "0",
                  12,
                )}{" "}{translateUi("credits")}</TableCell>
              <TableCell className="capitalize">
                {schedule.recurrence}
              </TableCell>
              <TableCell>{expiryLabel(schedule.expiry)}</TableCell>
              <TableCell>
                <div>{billingTargetLabel(schedule)}</div>
                <p className="text-13 text-muted-foreground">
                  {schedule.scope.all_services
                    ? translateUi("All services")
                    : schedule.scope.service_slugs.join(", ")}
                </p>
                {schedule.service_limits?.map((rule) => (
                  <p
                    key={rule.key}
                    className="max-w-sm whitespace-normal break-words text-13"
                  >
                    {rule.label || translateUi("Service limit")}:{" "}
                    {rule.service_slugs.join(", ")}{translateUi(" · up to")}{" "}
                    <span className="break-all">
                      {formatExactCredits(rule.limit, 12)}
                    </span>{" "}{translateUi("credits per issuance")}</p>
                ))}
                <RolloutSummary schedule={schedule} />
              </TableCell>
              <TableCell>
                <div>{periodLabel(schedule)}</div>
                {schedule.skipped_periods > 0 ? (
                  <Badge variant="warning" className="mt-1">
                    {formatNumber(schedule.skipped_periods)}{translateUi(" skipped period")}{schedule.skipped_periods === 1 ? "" : "s"}
                  </Badge>
                ) : null}
              </TableCell>
              <TableCell>
                <Badge variant={schedule.is_active ? "success" : "secondary"}>
                  {schedule.is_active ? translateUi("Active") : translateUi("Paused")}
                </Badge>
              </TableCell>
              {canWrite ? (
                <TableCell className="text-right">
                  <div className="flex items-center justify-end gap-3">
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      title={translateUi("Edit schedule")}
                      aria-label={translateUi("Edit schedule")}
                      disabled={updatePending}
                      onClick={() => onEdit(schedule)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Switch
                      checked={schedule.is_active}
                      disabled={updatePending}
                      aria-label={
                        schedule.is_active
                          ? translateUi("Pause schedule")
                          : translateUi("Resume schedule")
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
              >{translateUi("No credit schedules.")}</TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </div>
  );
}

function RolloutSummary({ schedule }: { readonly schedule: CreditSchedule }) {
  useUiTranslation();
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
            {disabled}{translateUi(" rollout off")}</Badge>
        </span>
      </TooltipTrigger>
      <TooltipContent>{translateUi("Selected owners outside the billing rollout cannot see billing yet.")}</TooltipContent>
    </Tooltip>
  );
}

function expiryLabel(expiry: CreditExpiryPolicy) {
  switch (expiry.kind) {
    case "end_of_period":
      return translateUi("End of period");
    case "after_days":
      return `After ${String(expiry.days)} day${expiry.days === 1 ? "" : "s"}`;
    case "never":
      return translateUi("Never");
  }
}

function periodLabel(schedule: CreditSchedule) {
  const period = schedule.current_period;
  if (!period) return schedule.is_active ? translateUi("Pending") : translateUi("No open period");
  if (period.status === "disbursing") {
    return translateUi("Disbursing {{amount}}", { amount: formatNumber(period.disbursed_count) });
  }
  return `Complete - ${formatNumber(period.disbursed_count)} - expires ${
    period.expires_at ? formatDate(period.expires_at) : "never"
  }`;
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat(getFormattingLocale(), {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  }).format(new Date(value));
}

function formatNumber(value: number) {
  return new Intl.NumberFormat(getFormattingLocale()).format(value);
}
