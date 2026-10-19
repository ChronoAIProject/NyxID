import type { BillingTargetKind } from "@/schemas/billing-credits";

type Targets = {
  readonly target_kind: BillingTargetKind;
  readonly target_user_ids?: readonly string[];
  readonly target_org_ids?: readonly string[];
  readonly target_group_ids?: readonly string[];
};

export function billingTargetLabel(target: Targets): string {
  const count = (ids: readonly string[] | undefined, singular: string) =>
    `${String(ids?.length ?? 0)} ${singular}${ids?.length === 1 ? "" : "s"}`;
  switch (target.target_kind) {
    case "all_users":
      return "All owners";
    case "selected_users":
      return target.target_user_ids
        ? `${String(target.target_user_ids.length)} selected`
        : "Selected owner";
    case "org_members":
      return `${count(target.target_org_ids, "organization")} · members`;
    case "groups":
      return `${count(target.target_group_ids, "group")} · members`;
  }
}

export function normalizedBillingTargets(target: Targets) {
  const ids = (
    kind: BillingTargetKind,
    values: readonly string[] | undefined,
  ) => (target.target_kind === kind ? [...new Set(values ?? [])].sort() : []);
  return {
    target_user_ids: ids("selected_users", target.target_user_ids),
    target_org_ids: ids("org_members", target.target_org_ids),
    target_group_ids: ids("groups", target.target_group_ids),
  };
}
