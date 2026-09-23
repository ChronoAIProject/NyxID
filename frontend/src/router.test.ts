import { describe, expect, it } from "vitest";
import { channelBotSetupRoute, loginAgentKeyRoute, loginDeviceRoute, loginCodeRoute, oauthCompleteRoute } from "./router";
import { isPublicPath } from "./lib/public-paths";

describe.each([loginAgentKeyRoute, loginDeviceRoute])("Login approval route $fullPath", (route) => {
  it("is public and accepts a user code without starting a preview", () => {
    expect(isPublicPath(route.fullPath)).toBe(true);
    const validate = route.options.validateSearch;
    if (typeof validate !== "function") throw new Error("Expected a search validator");
    expect(validate({ user_code: "2-abcd-efgh", ignored: "value" })).toEqual({ user_code: "2-abcd-efgh" });
  });

  it("accepts missing and malformed search without throwing", () => {
    const validate = route.options.validateSearch;
    if (typeof validate !== "function") throw new Error("Expected a search validator");
    expect(validate({})).toEqual({});
    expect(validate({ user_code: "garbage!" })).toEqual({ user_code: "garbage!" });
    for (const user_code of [null, 123, ["ABCD-EFGH"], {}]) {
      expect(validate({ user_code })).toEqual({ user_code: "" });
    }
  });
});

it("keeps login-code minting free of code input parameters", () => {
  const validate = loginCodeRoute.options.validateSearch;
  if (typeof validate !== "function") throw new Error("Expected a search validator");
  expect(validate({ user_code: "ABCD-EFGH" })).toEqual({});
});

describe("OAuth completion route registration", () => {
  it("registers the completion component outside the backend OAuth namespace", () => {
    expect(oauthCompleteRoute.fullPath).toBe("/oauth-complete");
  });
});

it("protects per-platform setup routes and accepts scalar prefill values", () => {
  expect(channelBotSetupRoute.fullPath).toBe("/channel-bots/connect/$platform");
  expect(isPublicPath("/channel-bots/connect/telegram")).toBe(false);
  const validate = channelBotSetupRoute.options.validateSearch;
  if (typeof validate !== "function") throw new Error("Expected a search validator");
  expect(validate({ label: "a".repeat(150), target_org_id: ["org"], request_id: "invalid", bot_token: "private", future_field: "value", nested: { value: "ignored" } }))
    .toEqual({ label: "a".repeat(128), target_org_id: undefined, request_id: undefined, bot_token: "private", future_field: "value" });
});
