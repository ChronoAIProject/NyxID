import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { exactCredits, formatExactCredits } from "@/lib/credits";
import type { CreditGrant } from "@/schemas/billing-credits";
import { DialogDescription } from "@/components/ui/dialog";

export function GrantRevokeDescription({
  grant,
}: {
  readonly grant: CreditGrant;
}) {
  useUiTranslation();
  return (
    <DialogDescription className="space-y-2">
      <span className="block">{translateUi("Reclaim the unused part of the remaining")}{" "}
        {formatCredits(
          exactCredits(grant.remaining, grant.remaining_micros) ?? "0",
        )}{" "}{translateUi("for")}{" "}
        {grant.recipient_display_name ||
          grant.recipient_email ||
          translateUi("this recipient")}{translateUi("? Held requests finish first; only the final unused remainder is reclaimed. Other allocations and purchased credits are unaffected. This cannot be undone.")}</span>
      <span className="block break-all text-13">{translateUi("Allocation ")}{grant.id}{translateUi(" · created")}{" "}
        {new Date(grant.created_at).toLocaleString(getFormattingLocale())}
      </span>
      {grant.schedule_id ? (
        <span className="block">{translateUi("Schedule ")}{grant.schedule_id}
          {grant.period_start
            ? translateUi(" · period starting {{value1}}", { value1: new Date(grant.period_start).toLocaleString(getFormattingLocale()) })
            : ""}
          {translateUi(". This reclaim affects this allocation only. Pausing the schedule stops future issuance. ")}</span>
      ) : null}
    </DialogDescription>
  );
}

function formatCredits(value: string) {
  return `${formatExactCredits(value, 12)} credits`;
}
