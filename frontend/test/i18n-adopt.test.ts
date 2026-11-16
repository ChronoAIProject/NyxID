import { describe, expect, it } from "vitest";
import { applyDelta, scanUsage, stripPluralSuffix } from "../scripts/lib/i18n-catalogs.mjs";

describe("legacy i18n delta adoption", () => {
  it("applies independent additions, base changes, and removals", () => {
    const result = applyDelta(
      { "Base value": "old", "Removed value": "gone" },
      { "Base value": "new", "Added value": "added" },
      { "Base value": "old", "Removed value": "gone", "Onto value": "kept" },
    );
    expect(result.conflicts).toEqual([]);
    expect(result.result).toEqual({ "Base value": "new", "Added value": "added", "Onto value": "kept" });
  });

  it("reports a conflict when onto changed the same base value", () => {
    const result = applyDelta({ Key: "base" }, { Key: "branch" }, { Key: "onto" }, "en");
    expect(result.result).toEqual({ Key: "onto" });
    expect(result.conflicts).toEqual(["en: changed key diverged: Key"]);
  });

  it("co-locates plural siblings and promotes keys used by several areas", () => {
    const usage = scanUsage(
      [
        ["src/components/assistant/chat-view.tsx", `t("Requests_one"); t("Requests_other"); t("Shared");`],
        ["src/pages/billing.tsx", `t("Shared");`],
      ],
      ["Requests_one", "Requests_other", "Shared", "Unseen"],
    );
    expect(stripPluralSuffix("Requests_one")).toBe("Requests");
    expect(usage.get("Requests")).toBe("assistant-chat");
    expect(usage.get("Shared")).toBe("shared");
    expect(usage.get("Unseen")).toBe("unattributed");
  });
});
