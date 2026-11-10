import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { credits } from "@/lib/billing-display";
import type { CreditGrant } from "@/schemas/billing-credits";
import { DialogDescription } from "@/components/ui/dialog";

export function GrantRevokeDescription({
  grant,
}: {
  readonly grant: CreditGrant;
}) {
  const { t } = useUiTranslation();
  return (
    <DialogDescription className="space-y-2">
      <span className="block">{t("Revoke the remaining")}{" "}
        {formatCredits(grant.remaining ?? grant.remaining_micros)}{t(" for")}{" "}
        {grant.recipient_display_name ||
          grant.recipient_email ||
          t("this recipient")}{t("? This cannot be undone.")}</span>
      {grant.schedule_id ? (
        <span className="block">{t("This revocation affects this period only. Pausing the schedule stops future disbursements.")}</span>
      ) : null}
    </DialogDescription>
  );
}

function formatCredits(value: string | number) {
  return translateUi("{{value}} credits", { value: credits(value) });
}
