import { describe, expect, it } from "vitest";
import {
  FEATURE_FLAG,
  isGlobalOnlyFlag,
  isUserOnlyFlag,
  userHasFeature,
} from "./feature-flags";

it("restricts shared CMA rate synchronization to global overrides", () => {
  expect(isGlobalOnlyFlag(FEATURE_FLAG.CMA_SANDBOX_RATE_SYNC)).toBe(true);
  expect(isGlobalOnlyFlag(FEATURE_FLAG.CMA_SANDBOX_BILLING)).toBe(false);
});

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

describe("rollout scope restrictions", () => {
  it("marks execution permissions as per-person only", () => {
    expect(isUserOnlyFlag(FEATURE_FLAG.EXECUTION_PERMISSIONS)).toBe(true);
    expect(isGlobalOnlyFlag(FEATURE_FLAG.EXECUTION_PERMISSIONS)).toBe(false);
  });

  it("leaves ordinary and global-only flags unrestricted per person", () => {
    expect(isUserOnlyFlag(FEATURE_FLAG.NYXAGENT_ENGINE)).toBe(false);
    expect(isUserOnlyFlag(FEATURE_FLAG.INVITATION_CODE)).toBe(false);
  });
});
