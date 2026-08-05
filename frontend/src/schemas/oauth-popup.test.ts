import { describe, expect, it } from "vitest";
import {
  isOAuthRetryMessage,
  oauthCompletionSearchSchema,
  validateAuthorizationUrl,
} from "./oauth-popup";
import { OAUTH_ERROR_CODES, OAUTH_FLOW_TOKENS } from "@/types/oauth-popup";

const NONCE = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";

describe("OAuth popup schemas", () => {
  it("pins flow tokens and parses every completion error", () => {
    expect(OAUTH_FLOW_TOKENS).toEqual(["cc", "kc", "pc", "sa", "wz", "sl"]);
    for (const code of OAUTH_ERROR_CODES) {
      expect(
        oauthCompletionSearchSchema.parse({
          status: "error",
          flow: "cc",
          code,
          nonce: NONCE,
        }),
      ).toEqual({ status: "error", flow: "cc", code, nonce: NONCE });
    }
  });

  it("degrades unknown tokens and missing status to a direct-open shape", () => {
    expect(
      oauthCompletionSearchSchema.parse({
        flow: "future",
        code: "future",
        nonce: "bad",
      }),
    ).toEqual({
      status: undefined,
      flow: undefined,
      code: undefined,
      nonce: undefined,
    });
  });

  it("accepts real cross-origin providers only with exact state binding", () => {
    const valid = `https://github.com/login/oauth/authorize?client_id=x&state=1cc_${NONCE}`;
    expect(validateAuthorizationUrl(valid, NONCE)?.href).toBe(valid);
    expect(
      isOAuthRetryMessage({
        type: "oauth_retry",
        nextNonce: NONCE,
        url: valid,
      }),
    ).toBe(true);

    for (const url of [
      "javascript:alert(1)",
      `https://user:pass@github.com/login/oauth/authorize?state=1cc_${NONCE}`,
      "https://github.com/login/oauth/authorize",
      `https://github.com/login/oauth/authorize?state=1cc_00000000-0000-4000-8000-000000000000`,
    ]) {
      expect(validateAuthorizationUrl(url, NONCE)).toBeNull();
    }
  });

  it("binds retry navigation to the interstitial's provider origin", () => {
    const githubUrl = `https://github.com/login/oauth/authorize?state=1cc_${NONCE}`;
    expect(
      validateAuthorizationUrl(githubUrl, NONCE, "https://github.com")?.href,
    ).toBe(githubUrl);
    expect(
      isOAuthRetryMessage(
        { type: "oauth_retry", nextNonce: NONCE, url: githubUrl },
        "https://github.com",
      ),
    ).toBe(true);
    expect(
      validateAuthorizationUrl(
        `https://evil.example/oauth?state=1cc_${NONCE}`,
        NONCE,
        "https://github.com",
      ),
    ).toBeNull();
  });
});
