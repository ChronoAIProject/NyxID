import { describe, expect, it } from "vitest";
import {
  createServiceAccountSchema,
  updateServiceAccountSchema,
} from "./service-accounts";

const base = {
  name: "qa-sa",
  description: "",
  allowed_scopes: "llm:proxy",
  role_ids: "",
};

describe("createServiceAccountSchema rate_limit_override", () => {
  it("accepts empty (no override) and positive integers", () => {
    for (const rate_limit_override of ["", "1", "500"]) {
      expect(
        createServiceAccountSchema.safeParse({ ...base, rate_limit_override })
          .success,
        rate_limit_override,
      ).toBe(true);
    }
  });

  it("rejects non-numeric, zero, negative, and fractional overrides", () => {
    for (const rate_limit_override of ["abc", "0", "-5", "1.5"]) {
      expect(
        createServiceAccountSchema.safeParse({ ...base, rate_limit_override })
          .success,
        rate_limit_override,
      ).toBe(false);
    }
  });
});

describe("updateServiceAccountSchema rate_limit_override", () => {
  it("applies the same integer rule", () => {
    expect(
      updateServiceAccountSchema.safeParse({
        ...base,
        rate_limit_override: "abc",
        is_active: true,
      }).success,
    ).toBe(false);
  });
});
