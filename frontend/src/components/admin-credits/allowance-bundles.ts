import type {
  AllowanceBundleForm,
  UsageAllowance,
} from "@/schemas/billing-credits";
import { normalizedBillingTargets } from "@/lib/billing-targets";

export type AllowanceBundle = UsageAllowance & { rows: UsageAllowance[] };
export function groupAllowances(
  rows: readonly UsageAllowance[],
): AllowanceBundle[] {
  const bundles = new Map<string, AllowanceBundle>();
  for (const row of rows) {
    const key = row.bundle_id ?? row.id;
    const group = bundles.get(key);
    if (group) group.rows.push(row);
    else bundles.set(key, { ...row, id: key, rows: [row] });
  }
  return [...bundles.values()].map((group) => ({
    ...group,
    is_active: group.rows.some((r) => r.is_active),
  }));
}
export function bundleStatus(bundle: AllowanceBundle) {
  return bundle.rows.every((r) => r.is_active)
    ? "Active"
    : bundle.is_active
      ? "Partially disabled"
      : "Disabled";
}
export function bundleForm(bundle: AllowanceBundle): AllowanceBundleForm {
  const activeRows = bundle.rows.filter((row) => row.is_active);
  return {
    service_ref: bundle.service_id,
    target_kind: bundle.target_kind,
    ...normalizedBillingTargets(bundle),
    units: (activeRows.length ? activeRows : bundle.rows).map(
      ({ metric, quantity, recurrence }) => ({
        metric,
        quantity,
        recurrence,
      }),
    ),
  };
}
