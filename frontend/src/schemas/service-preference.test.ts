import { describe, expect, it } from "vitest";
import {
  servicePreferenceRequestSchema as request,
  servicePreferenceResponseSchema as response,
} from "./service-preference";
const id = "11111111-1111-4111-8111-111111111111";
describe("service preference contract", () => {
  it("bounds IDs, duplicates, versions and unknown fields", () => {
    for (const body of [
      { ordered: Array<string>(201).fill(id), expected_version: 0 },
      { ordered: ["invalid"], expected_version: 0 },
      { ordered: ["AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"], expected_version: 0 },
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
  it("accepts the absent-document response", () => {
    expect(
      response.parse({ ordered: [], version: 0, updated_at: null }),
    ).toEqual({ ordered: [], version: 0, updated_at: null });
  });
});
