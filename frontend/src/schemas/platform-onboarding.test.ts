import { expect, it } from "vitest";
import {
  blankFacts,
  onboardingSearch,
  setupFactsSchema,
  setupLimitsSchema,
  setupStatus,
} from "./platform-onboarding";
it("accepts bounded hints without URL authority", () => {
  expect(
    onboardingSearch({
      template: "restaurant-starter",
      code: "ABCD1234",
      owner_id: "other",
      approve: true,
    }),
  ).toEqual({ template: "restaurant-starter", code: "ABCD1234" });
  expect(
    onboardingSearch({ template: "workflow", code: "x".repeat(100) }),
  ).toEqual({});
});
it("rejects hidden fields and credential URLs", () => {
  const facts = { ...blankFacts(), name: "Cafe" };
  expect(
    setupFactsSchema.safeParse({ ...facts, grants: ["all"] }).success,
  ).toBe(false);
  expect(
    setupFactsSchema.safeParse({
      ...facts,
      reservation_url: "https://user:secret@example.com",
    }).success,
  ).toBe(false);
});
it("retains exact caps", () => {
  const limits = {
    daily_credits: "1.000000000001",
    turns_per_hour: 10,
    concurrent_turns: 1,
    max_input_chars: 4096,
    max_output_tokens: 2048,
  };
  expect(setupLimitsSchema.parse(limits).daily_credits).toBe("1.000000000001");
  expect(
    setupLimitsSchema.safeParse({ ...limits, daily_credits: "1e6" }).success,
  ).toBe(false);
  for (const daily_credits of ["0", "0.000000000000", "1000.000000000001", "1001"]) {
    expect(setupLimitsSchema.safeParse({ ...limits, daily_credits }).success).toBe(false);
  }
  for (const daily_credits of ["0.000000000001", "1000.000000000000"]) {
    expect(setupLimitsSchema.safeParse({ ...limits, daily_credits }).success).toBe(true);
  }
  expect(Object.keys(setupStatus)).toHaveLength(12);
});
