import { expect, it } from "vitest";
import { billingRow } from "@/test/billing-fixture";
import { billingUsageRowSchema } from "@/schemas/billing";
import { groupRows } from "./billing-usage";

it("groups usage by service operation and labels base-priced requests", () => {
  const catalog = [{ slug: "api-twitter", name: "X", inference: null }];
  const row = (operation: string | null, estimated: number) =>
    billingUsageRowSchema.parse(
      billingRow({
        service_slug: "api-twitter",
        service_id: "x",
        metric: "requests",
        lago_metric_code: operation
          ? `platform_svc_api-twitter_byok_op_${operation}`
          : "platform_svc_api-twitter_byok",
        operation,
        estimated_credits_micros: estimated,
      }),
    );
  const groups = groupRows(
    catalog,
    [row("get_me", 10), row("get_me", 10), row(null, 5)],
    "operation",
  );
  expect(groups.map((group) => [group.name, group.rows.length])).toEqual([
    ["X · get_me", 2],
    ["X · Base price", 1],
  ]);
  // Rows from servers without the field still parse and group as base price.
  const { operation: _omitted, ...older } = row(null, 1);
  void _omitted;
  expect(
    groupRows(catalog, [billingUsageRowSchema.parse(older)], "operation")[0]!
      .name,
  ).toBe("X · Base price");
});
