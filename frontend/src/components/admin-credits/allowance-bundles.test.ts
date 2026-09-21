import { describe, expect, it } from "vitest";
import {
  allowanceBundleFormSchema,
  type UsageAllowance,
} from "@/schemas/billing-credits";
import { bundleForm, bundleStatus, groupAllowances } from "./allowance-bundles";
const row: UsageAllowance = {
  id: "legacy",
  service_id: "service",
  service_slug: "service",
  metric: "tokens",
  quantity: 100,
  recurrence: "daily",
  target_kind: "all_users",
  target_user_ids: [],
  is_active: true,
  created_by: "admin",
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
};
describe("allowance bundles", () => {
  it("groups legacy singletons and mixed status bundles without hiding disabled units", () => {
    const groups = groupAllowances([
      row,
      { ...row, id: "a", bundle_id: "bundle" },
      {
        ...row,
        id: "b",
        bundle_id: "bundle",
        metric: "images",
        is_active: false,
      },
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0]?.id).toBe("legacy");
    expect(bundleStatus(groups[0]!)).toBe("Active");
    expect(bundleStatus(groups[1]!)).toBe("Partially disabled");
    expect(bundleForm(groups[1]!).units.map((u) => u.metric)).toEqual([
      "tokens",
    ]);
  });
  it("loads all units when the whole bundle is disabled so saving can re-enable it", () => {
    const bundle = groupAllowances([
      { ...row, bundle_id: "bundle", is_active: false },
      {
        ...row,
        id: "images",
        bundle_id: "bundle",
        metric: "images",
        is_active: false,
      },
    ])[0]!;
    expect(bundleStatus(bundle)).toBe("Disabled");
    const form = bundleForm(bundle);
    expect(form.units.map((unit) => unit.metric)).toEqual(["tokens", "images"]);
    expect(allowanceBundleFormSchema.safeParse(form).success).toBe(true);
  });
  it("rejects duplicate, empty, excessive and invalid units", () => {
    const form = bundleForm(groupAllowances([row])[0]!);
    expect(allowanceBundleFormSchema.safeParse(form).success).toBe(true);
    for (const units of [
      [],
      [...form.units, ...form.units],
      Array(17).fill(form.units[0]),
      [{ ...form.units[0], quantity: 0 }],
    ]) {
      expect(
        allowanceBundleFormSchema.safeParse({ ...form, units }).success,
      ).toBe(false);
    }
  });
});
