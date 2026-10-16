import { describe, expect, it } from "vitest";
import { publicationFailureText, publicationRuleText } from "./skill-publication-copy";

describe("sanitized publication copy", () => {
  it.each(["frontmatter_tag", "frontmatter_runtime_env_var", "frontmatter_depends_on"])("uses plural grammar for %s", (rule) => {
    expect(publicationRuleText(rule)).toContain(" do not meet ");
  });
  it("keeps unknown rule text private", () => {
    expect(publicationRuleText("PRIVATE_SENTINEL")).toBe("Ornn reported an additional format rule.");
  });
  it("claims a repair notification only for authored proposals", () => {
    expect(publicationFailureText("ornn_validation_failed", "publication_failed", "authored")).toContain("The agent that drafted it was asked");
    expect(publicationFailureText("ornn_validation_failed", "publication_failed", "learned")).toContain("Deny or discard");
  });
});

it("gives the same actionable root-items guidance as the backend", () => {
  expect(publicationRuleText("allowed_root_items")).toBe("Keep supporting files under references/.");
});
