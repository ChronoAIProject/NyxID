import { describe, expect, it } from "vitest";
import { parseAdditionalScopes } from "./parse-additional-scopes";

// Pins the splitter semantics shared by the dashboard add-key dialog
// and the CLI pair wizard (NyxID#917). Must stay in lockstep with the
// backend's `parse_additional_scopes` and the CLI `--scope` flag:
// comma / whitespace separated, trimmed, deduped, order-preserving.
describe("parseAdditionalScopes", () => {
  it("returns an empty list for empty or whitespace-only input", () => {
    expect(parseAdditionalScopes("")).toEqual([]);
    expect(parseAdditionalScopes("   \n\t ")).toEqual([]);
  });

  it("splits on commas", () => {
    expect(parseAdditionalScopes("media.write,tweet.read")).toEqual([
      "media.write",
      "tweet.read",
    ]);
  });

  it("splits on spaces and newlines", () => {
    expect(parseAdditionalScopes("repo read:org\nuser:email")).toEqual([
      "repo",
      "read:org",
      "user:email",
    ]);
  });

  it("trims surrounding whitespace and collapses mixed separators", () => {
    expect(parseAdditionalScopes("  media.write ,  tweet.read , ")).toEqual([
      "media.write",
      "tweet.read",
    ]);
  });

  it("dedupes while preserving first-seen order", () => {
    expect(
      parseAdditionalScopes("media.write,tweet.read,media.write"),
    ).toEqual(["media.write", "tweet.read"]);
  });
});
