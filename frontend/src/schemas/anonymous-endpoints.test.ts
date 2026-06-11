import { describe, expect, it } from "vitest";
import { anonymousEndpointCreateSchema } from "./anonymous-endpoints";

describe("anonymous endpoint schemas", () => {
  it("normalizes paths and coerces daily quota", () => {
    const parsed = anonymousEndpointCreateSchema.parse({
      enabled: true,
      method: "GET",
      path_pattern: "public/**",
      daily_quota: "10",
    });

    expect(parsed).toEqual({
      enabled: true,
      method: "GET",
      path_pattern: "/public/**",
      daily_quota: 10,
    });
  });

  it("rejects wildcards outside the trailing segment", () => {
    expect(() =>
      anonymousEndpointCreateSchema.parse({
        enabled: true,
        method: "GET",
        path_pattern: "/public/*/items",
        daily_quota: 10,
      }),
    ).toThrow(/Wildcard/);
  });

  it("rejects dot segments and zero quotas", () => {
    expect(() =>
      anonymousEndpointCreateSchema.parse({
        enabled: true,
        method: "GET",
        path_pattern: "/public/../secret",
        daily_quota: 1,
      }),
    ).toThrow(/dot segments/);

    expect(() =>
      anonymousEndpointCreateSchema.parse({
        enabled: true,
        method: "GET",
        path_pattern: "/public",
        daily_quota: 0,
      }),
    ).toThrow(/at least 1/);
  });
});
