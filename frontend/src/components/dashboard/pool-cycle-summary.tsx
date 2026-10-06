import type { ServicePoolMember } from "@/schemas/pools";
import { validPoolWeight } from "./pool-editor-state";

export function PoolCycleSummary({
  members,
  priority,
  weighted,
  label,
}: {
  members: Partial<ServicePoolMember>[];
  priority: boolean;
  weighted: boolean;
  label: (id: string, index: number) => string;
}) {
  const groups = new Map<
    number,
    { member: Partial<ServicePoolMember>; index: number }[]
  >();
  members.forEach((member, index) => {
    if (member.enabled === false) return;
    const tier = priority ? (member.priority ?? 0) : 0;
    if (!Number.isInteger(tier) || tier < 0 || tier > 4294967295) return;
    const group = groups.get(tier) ?? [];
    group.push({ member, index });
    groups.set(tier, group);
  });
  return (
    <div className="space-y-2 rounded-xl border border-border/50 bg-muted/20 p-3 text-[11px] text-muted-foreground">
      <p className="font-medium text-foreground">
        {priority
          ? "Cycles within each priority"
          : weighted
            ? "Repeating weighted cycle"
            : "Repeating round-robin cycle"}
      </p>
      <p>
        {weighted
          ? "Each enabled connection takes consecutive turns equal to its weight, in the saved order."
          : "Each enabled connection gets one turn in the saved order."}{" "}
        Unavailable connections can be skipped.
        {priority
          ? " Lower priority numbers are tried first. Saving restarts each tier’s cycle."
          : " The current position is retained between requests and when saving; this does not promise a next or fallback connection."}
      </p>
      {groups.size === 0 && (
        <p>
          {priority
            ? "No enabled connections with a valid priority."
            : "No enabled connections."}
        </p>
      )}
      {[...groups]
        .sort(([a], [b]) => a - b)
        .map(([tier, group]) => (
          <p key={tier} className="break-words">
            {priority ? `Priority ${tier} cycle order: ` : "Cycle order: "}
            {group
              .map(({ member, index }) => {
                const name = label(member.user_service_id!, index);
                if (!weighted) return name;
                const weight = member.weight ?? 1;
                return `${name} (${validPoolWeight(weight) ? `${weight} ${weight === 1 ? "turn" : "turns"}` : "weight required"})`;
              })
              .join(" → ")}
          </p>
        ))}
    </div>
  );
}
