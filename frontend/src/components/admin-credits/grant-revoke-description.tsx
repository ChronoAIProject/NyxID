import { credits } from "@/lib/billing-display";
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
        Revoke the remaining{" "}
        {formatCredits(grant.remaining ?? grant.remaining_micros)} for{" "}
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

function formatCredits(value: string | number) {
  return `${credits(value)} credits`;
}
