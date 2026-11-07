import { expect, it } from "vitest";
import { billingRow } from "@/test/billing-fixture";
import { billingUsageRowSchema } from "@/schemas/billing";
import { groupRows, metricFamily } from "./billing-usage";

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

it("identifies org-funded channel guest usage without exposing the sender", () => {
  const base = billingUsageRowSchema.parse(
    billingRow({ api_key_id: "member-key", api_key_name: "Member agent" }),
  );
  const guest = billingUsageRowSchema.parse({
    ...base,
    api_key_id: "guest-key",
    org_channel_guest: {
      channel_id: "12345678-binding",
      sender_hash: "private-sender-hash",
    },
  });
  const groups = groupRows(
    [],
    [base, guest, { ...guest, api_key_id: "setup-key" }],
    "agent",
  );
  expect(groups.map((group) => group.name)).toContain("Member agent");
  const visitors = groups.find(
    (group) => group.name === "Channel guests · 12345678",
  );
  expect(visitors?.rows).toHaveLength(2);
  expect(
    groups.every((group) => !group.name.includes("private-sender-hash")),
  ).toBe(true);
});

it("shows strict model token units in the token usage and allowance group", () => {
  for (const metric of [
    "reasoning_tokens",
    "image_input_tokens",
    "image_output_tokens",
    "audio_input_tokens",
    "audio_output_tokens",
  ]) {
    expect(metricFamily(metric)).toBe("Tokens");
  }
});
