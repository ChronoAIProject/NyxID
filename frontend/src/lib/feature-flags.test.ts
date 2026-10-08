import { describe, expect, it } from "vitest";
import { FEATURE_FLAG, userHasFeature } from "./feature-flags";

describe("userHasFeature", () => {
  it.each([null, undefined])(
    "returns false for a missing user (%s)",
    (user) => {
      expect(userHasFeature(user, FEATURE_FLAG.NYXAGENT_ENGINE)).toBe(false);
    },
  );

  it("returns false for missing capabilities", () => {
    expect(userHasFeature({}, FEATURE_FLAG.NYXAGENT_ENGINE)).toBe(false);
  });

  it("returns false for a missing feature list", () => {
    expect(
      userHasFeature({ capabilities: {} }, FEATURE_FLAG.NYXAGENT_ENGINE),
    ).toBe(false);
  });

  it("returns true when the requested feature is present", () => {
    expect(
      userHasFeature(
        { capabilities: { enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE] } },
        FEATURE_FLAG.NYXAGENT_ENGINE,
      ),
    ).toBe(true);
  });

  it("returns false when the requested feature is absent", () => {
    expect(
      userHasFeature(
        {
          capabilities: { enabled_features: [FEATURE_FLAG.DIRECT_CHAT_ENGINE] },
        },
        FEATURE_FLAG.NYXAGENT_ENGINE,
      ),
    ).toBe(false);
    expect(
      userHasFeature(
        { capabilities: { enabled_features: [] } },
        FEATURE_FLAG.NYXAGENT_ENGINE,
      ),
    ).toBe(false);
  });
});
