import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
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
  const { t } = useUiTranslation();
  return (
    <div className="overflow-x-auto rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("Amount")}</TableHead>
            <TableHead>{t("Recurrence")}</TableHead>
            <TableHead>{t("Expiry")}</TableHead>
            <TableHead>{t("Targets")}</TableHead>
            <TableHead>{t("Current period")}</TableHead>
            <TableHead>{t("Status")}</TableHead>
            {canWrite ? (
              <TableHead className="text-right">{t("Actions")}</TableHead>
            ) : null}
          </TableRow>
        </TableHeader>
        <TableBody>
          {schedules.map((schedule) => (
            <TableRow key={schedule.id}>
              <TableCell className="font-medium">
                {formatExactCredits(
                  exactCredits(
                    schedule.amount,
                    schedule.amount_credits,
                    "whole",
                  ) ?? "0",
                )}{" "}{t("credits")}</TableCell>
              <TableCell className="capitalize">
                {schedule.recurrence}
              </TableCell>
              <TableCell>{expiryLabel(schedule.expiry)}</TableCell>
              <TableCell>
                <div>{billingTargetLabel(schedule)}</div>
                <RolloutSummary schedule={schedule} />
              </TableCell>
              <TableCell>
                <div>{periodLabel(schedule)}</div>
                {schedule.skipped_periods > 0 ? (
                  <Badge variant="warning" className="mt-1">
                    {formatNumber(schedule.skipped_periods)}{t(" skipped period")}{schedule.skipped_periods === 1 ? "" : "s"}
                  </Badge>
                ) : null}
              </TableCell>
              <TableCell>
                <Badge variant={schedule.is_active ? "success" : "secondary"}>
                  {schedule.is_active ? t("Active") : t("Paused")}
                </Badge>
              </TableCell>
              {canWrite ? (
                <TableCell className="text-right">
                  <div className="flex items-center justify-end gap-3">
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      title={t("Edit schedule")}
                      onClick={() => onEdit(schedule)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Switch
                      checked={schedule.is_active}
                      disabled={updatePending}
                      aria-label={
                        schedule.is_active
                          ? t("Pause schedule")
                          : t("Resume schedule")
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
              >{t("No credit schedules.")}</TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </div>
  );
}

function RolloutSummary({ schedule }: { readonly schedule: CreditSchedule }) {
  const { t } = useUiTranslation();
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
            {disabled}{t(" rollout off")}</Badge>
        </span>
      </TooltipTrigger>
      <TooltipContent>{t("Selected owners outside the billing rollout cannot see billing yet.")}</TooltipContent>
    </Tooltip>
  );
}

function expiryLabel(expiry: CreditExpiryPolicy) {
  switch (expiry.kind) {
    case "end_of_period":
      return translateUi("End of period");
    case "after_days":
      return translateUi(expiry.days === 1 ? "After {{count}} day" : "After {{count}} days", { count: expiry.days });
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
  return translateUi("Complete - {{amount}} - expires {{expiry}}", { amount: formatNumber(period.disbursed_count), expiry: period.expires_at ? formatDate(period.expires_at) : translateUi("never") });
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
