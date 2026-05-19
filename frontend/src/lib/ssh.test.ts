import { describe, expect, it } from "vitest";
import { parseAllowedPrincipals } from "./ssh";

describe("parseAllowedPrincipals", () => {
  it("deduplicates principals while preserving their original order", () => {
    expect(
      parseAllowedPrincipals("ubuntu, deploy\nubuntu\nadmin, deploy"),
    ).toEqual(["ubuntu", "deploy", "admin"]);
  });

  it("ignores blank entries", () => {
    expect(parseAllowedPrincipals("ubuntu,\n ,deploy")).toEqual([
      "ubuntu",
      "deploy",
    ]);
  });
});
