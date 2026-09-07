import { describe, expect, it } from "vitest";
import { loginAgentKeyRoute, oauthCompleteRoute } from "./router";
import { isPublicPath } from "./lib/public-paths";

describe("Agent Key login route", () => {
  it("is public and ignores user codes supplied in the browser URL", () => {
    expect(loginAgentKeyRoute.fullPath).toBe("/login/agent-key");
    expect(isPublicPath("/login/agent-key")).toBe(true);
    const validate = loginAgentKeyRoute.options.validateSearch;
    expect(typeof validate).toBe("function");
    if (typeof validate === "function")
      expect(validate({ user_code: "ABCD-EFGH" })).toEqual({});
  });
});

describe("OAuth completion route registration", () => {
  it("registers the completion component outside the backend OAuth namespace", () => {
    expect(oauthCompleteRoute.fullPath).toBe("/oauth-complete");
  });
});
