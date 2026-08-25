import type { CreditGrant } from "@/schemas/billing-credits";
import { DialogDescription } from "@/components/ui/dialog";

export function GrantRevokeDescription({
  grant,
}: {
  readonly grant: CreditGrant;
}) {
  return (
    <DialogDescription className="space-y-2">
      <span className="block">
        Revoke the remaining {formatCredits(grant.remaining_micros)} for{" "}
        {grant.recipient_display_name ||
          grant.recipient_email ||
          "this recipient"}
        ? This cannot be undone.
      </span>
      {grant.schedule_id ? (
        <span className="block">
          This revocation affects this period only. Pausing the schedule stops
          future disbursements.
        </span>
      ) : null}
    </DialogDescription>
  );
}

function formatCredits(micros: number) {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 }).format(micros / 1_000_000)} credits`;
}
