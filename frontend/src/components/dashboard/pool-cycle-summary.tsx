import { useUiTranslation } from "@/i18n/use-ui-translation";
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
  const { t } = useUiTranslation();
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
    <div className="space-y-2 rounded-xl border border-border/50 bg-muted/20 p-3 text-11 text-muted-foreground">
      <p className="font-medium text-foreground">
        {priority
          ? t("Cycles within each priority")
          : weighted
            ? t("Repeating weighted cycle")
            : t("Repeating round-robin cycle")}
      </p>
      <p>
        {weighted
          ? t("Each enabled connection takes consecutive turns equal to its weight, in the saved order.")
          : t("Each enabled connection gets one turn in the saved order.")}{" "}{t("Unavailable connections can be skipped.")}{priority
          ? t(" Lower priority numbers are tried first. Saving restarts each tier’s cycle.")
          : t(" The current position is retained between requests and when saving; this does not promise a next or fallback connection.")}
      </p>
      {groups.size === 0 && (
        <p>
          {priority
            ? t("No enabled connections with a valid priority.")
            : t("No enabled connections.")}
        </p>
      )}
      {[...groups]
        .sort(([a], [b]) => a - b)
        .map(([tier, group]) => (
          <p key={tier} className="break-words">
            {priority ? t("Priority {{tier}} cycle order: ", { tier: tier }) : t("Cycle order: ")}
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
