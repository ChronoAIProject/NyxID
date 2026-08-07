import { describe, expect, it } from "vitest";
import { oauthCompleteRoute } from "./router";

describe("OAuth completion route registration", () => {
  it("registers the completion component outside the backend OAuth namespace", () => {
    expect(oauthCompleteRoute.fullPath).toBe("/oauth-complete");
  });
});
