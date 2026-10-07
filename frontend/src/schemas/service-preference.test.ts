import { describe, expect, it } from "vitest";
import {
  serviceGroupKeySchema,
  releaseHiddenPreferenceRequestSchema,
  servicePreferenceGroupRequestSchema as request,
  servicePreferenceResponseSchema as response,
} from "./service-preference";
const id = "11111111-1111-4111-8111-111111111111";
describe("service preference contract", () => {
  it("bounds IDs, duplicates, versions and unknown fields", () => {
    for (const body of [
      { ordered: Array<string>(201).fill(id), expected_version: 0 },
      { ordered: ["invalid"], expected_version: 0 },
      {
        ordered: ["AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"],
        expected_version: 0,
      },
      { ordered: [id, id], expected_version: 0 },
      {
        ordered: ["11111111-1111-4111-1111-111111111111"],
        expected_version: 0,
      },
      { ordered: [], expected_version: -1 },
      { ordered: [], expected_version: Number.MAX_SAFE_INTEGER },
      { ordered: [], expected_version: 0, unexpected: true },
    ])
      expect(request.safeParse(body).success).toBe(false);
    expect(
      request.safeParse({ ordered: [id], expected_version: 0 }).success,
    ).toBe(true);
  });
  it("validates catalog groups and strict release bodies", () => {
    for (const invalid of [
      "00000000-0000-0000-0000-000000000000",
      "ffffffff-ffff-ffff-ffff-ffffffffffff",
      "aaaaaaaa-aaaa-9aaa-8aaa-aaaaaaaaaaaa",
      "aaaaaaaa-aaaa-4aaa-1aaa-aaaaaaaaaaaa",
      "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    ])
      expect(
        serviceGroupKeySchema.safeParse(`catalog:${invalid}`).success,
      ).toBe(false);
    for (let version = 1; version <= 8; version++)
      expect(
        serviceGroupKeySchema.safeParse(
          `catalog:aaaaaaaa-aaaa-${String(version)}aaa-8aaa-aaaaaaaaaaaa`,
        ).success,
      ).toBe(true);
    expect(
      serviceGroupKeySchema.safeParse(
        "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa",
      ).success,
    ).toBe(true);
    expect(serviceGroupKeySchema.safeParse(`connection:${id}`).success).toBe(
      false,
    );
    expect(
      releaseHiddenPreferenceRequestSchema.safeParse({
        expected_version: 0,
        clear: true,
      }).success,
    ).toBe(false);
  });
  it("accepts the absent-document response", () => {
    expect(
      response.parse({ groups: [], version: 0, updated_at: null }),
    ).toEqual({ groups: [], version: 0, updated_at: null });
  });
});
