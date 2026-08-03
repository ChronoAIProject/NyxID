import { describe, expect, it } from "vitest";
import {
  PROJECTION_BACKOFF_POLICY,
  nextBackoffDelay,
} from "./backoff";

describe("assistant projection backoff", () => {
  it("floors every delay when random returns zero", () => {
    expect(
      Array.from({ length: 12 }, (_, attempt) =>
        nextBackoffDelay(PROJECTION_BACKOFF_POLICY, attempt, () => 0),
      ),
    ).toEqual(Array.from({ length: 12 }, () => 250));
  });

  it("caps exponential delays and can span the policy deadline", () => {
    const delays = Array.from({ length: 12 }, (_, attempt) =>
      nextBackoffDelay(PROJECTION_BACKOFF_POLICY, attempt, () => 0.999999),
    );
    expect(Math.max(...delays)).toBeLessThanOrEqual(30_000);
    expect(delays.reduce((sum, delay) => sum + delay, 0)).toBeGreaterThan(
      PROJECTION_BACKOFF_POLICY.deadlineMs,
    );
  });

  it("does not let an out-of-range random sample escape policy bounds", () => {
    expect(nextBackoffDelay(PROJECTION_BACKOFF_POLICY, 0, () => -10)).toBe(
      250,
    );
    expect(nextBackoffDelay(PROJECTION_BACKOFF_POLICY, 20, () => 10)).toBe(
      30_000,
    );
  });
});
