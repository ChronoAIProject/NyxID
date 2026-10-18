import type { CreditGrant } from "@/schemas/billing-credits";
import { DialogDescription } from "@/components/ui/dialog";
import { formatMicrocredits } from "@/lib/credits";

export function GrantRevokeDescription({
  grant,
}: {
  readonly grant: CreditGrant;
}) {
  return (
    <DialogDescription className="space-y-2">
      <span className="block">
        Request reclaim of the remaining{" "}
        {formatMicrocredits(
          grant.remaining_micros_exact ?? String(grant.remaining_micros),
        )}{" "}
        for{" "}
        {grant.recipient_display_name ||
          grant.recipient_email ||
          "this recipient"}
        ? This cannot be undone.
      </span>
      <span className="block">
        New usage stops drawing from this allocation immediately. Other
        allocations and purchased credits are unaffected.
      </span>
      {BigInt(grant.reserved_micros_exact ?? String(grant.reserved_micros)) >
      0n ? (
        <span className="block">
          Requests already holding credits may finish. Their final charges
          reduce the amount reclaimed; the exact remainder is removed
          automatically after those holds finish.
        </span>
      ) : null}
      <span className="block space-y-0.5 text-[11px] text-muted-foreground">
        <span className="block">
          Allocated {new Date(grant.created_at).toLocaleString()}
        </span>
        <span className="block [overflow-wrap:anywhere]">
          Allocation {grant.id}
        </span>
        {grant.schedule_id ? (
          <span className="block [overflow-wrap:anywhere]">
            Recurring {grant.schedule_id}
            {grant.period_start
              ? ` · UTC period ${new Date(grant.period_start).toISOString()}`
              : ""}
          </span>
        ) : null}
      </span>
      {grant.schedule_id ? (
        <span className="block">
          This reclaim affects this allocation only. Pausing recurring credits
          stops future issuance.
        </span>
      ) : null}
    </DialogDescription>
  );
}
