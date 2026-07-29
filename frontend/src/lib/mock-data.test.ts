import { describe, expect, it } from "vitest";
import { getMockResponse } from "./mock-data";

function catalogSlugs(endpoint: string): string[] {
  const response = getMockResponse(endpoint) as {
    readonly entries: readonly { readonly slug: string }[];
  };
  return response.entries.map((entry) => entry.slug);
}

describe("mock catalog", () => {
  it("exposes the action contract slug only through the full catalog", () => {
    expect(catalogSlugs("/catalog")).not.toContain("api-github");
    expect(catalogSlugs("/catalog?include_all=true")).toContain("api-github");
  });
});
