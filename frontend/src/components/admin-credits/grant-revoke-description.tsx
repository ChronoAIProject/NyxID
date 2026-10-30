import { exactCredits, formatExactCredits } from "@/lib/credits";
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
        Reclaim the unused part of the remaining{" "}
        {formatCredits(
          exactCredits(grant.remaining, grant.remaining_micros) ?? "0",
        )}{" "}
        for{" "}
        {grant.recipient_display_name ||
          grant.recipient_email ||
          "this recipient"}
        ? Held requests finish first; only the final unused remainder is
        reclaimed. Other allocations and purchased credits are unaffected. This
        cannot be undone.
      </span>
      <span className="block break-all text-11">
        Allocation {grant.id} · created{" "}
        {new Date(grant.created_at).toLocaleString()}
      </span>
      {grant.schedule_id ? (
        <span className="block">
          Schedule {grant.schedule_id}
          {grant.period_start
            ? ` · period starting ${new Date(grant.period_start).toLocaleString()}`
            : ""}
          . This reclaim affects this allocation only. Pausing the schedule
          stops future issuance.
        </span>
      ) : null}
    </DialogDescription>
  );
}

function formatCredits(value: string) {
  return `${formatExactCredits(value, 12)} credits`;
}
