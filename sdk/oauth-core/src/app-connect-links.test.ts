import { describe, expect, it, vi } from "vitest";
import {
  NyxAppConnectLinksClient,
  parseAppConnectLinkCallback,
} from "./app-connect-links.js";
import { NyxAppConnectError } from "./requirements.js";

const callback =
  "https://app.example/callback?state=expected&status=completed&app_connect_link_id=link-id&grant_update_required=true";

describe("app connection repair", () => {
  it("creates an app-bound repair with caller state and no client selector", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValue(
        new Response(
          JSON.stringify({
            id: "link-id",
            connect_url: "https://id.example/connect/app/link-id#t=secret",
            expires_at: "2026-09-15T00:30:00Z",
          }),
        ),
      );
    const client = new NyxAppConnectLinksClient(
      "https://id.example",
      () => "user-token",
      fetchFn,
    );
    const created = await client.create({
      callbackUrl: "https://app.example/callback",
      state: "expected",
    });
    expect(created.id).toBe("link-id");
    expect(fetchFn).toHaveBeenCalledWith(
      "https://id.example/api/v1/app-connect-links",
      {
        method: "POST",
        headers: {
          Authorization: "Bearer user-token",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          callback_url: "https://app.example/callback",
          state: "expected",
        }),
      },
    );
  });

  it("requires a user token and nonempty state", async () => {
    const fetchFn = vi.fn<typeof fetch>();
    const client = new NyxAppConnectLinksClient(
      "https://id.example",
      () => undefined,
      fetchFn,
    );
    await expect(
      client.create({
        callbackUrl: "https://app.example/callback",
        state: "expected",
      }),
    ).rejects.toThrow("Missing access token");
    await expect(
      client.create({ callbackUrl: "https://app.example/callback", state: "" }),
    ).rejects.toThrow("state is required");
    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("parses a correlated callback without a token request", () => {
    expect(parseAppConnectLinkCallback(callback, "expected")).toEqual({
      appConnectLinkId: "link-id",
      state: "expected",
      status: "completed",
      grantUpdateRequired: true,
    });
  });

  it.each([
    "https://app.example/callback?error=access_denied",
    "https://app.example/callback?state=wrong&error=access_denied",
    "https://app.example/callback?state=expected&state=wrong&error=access_denied",
  ])("checks state before interpreting errors: %s", (url) => {
    expect(() => parseAppConnectLinkCallback(url, "expected")).toThrow(
      "state mismatch",
    );
    try {
      parseAppConnectLinkCallback(url, "expected");
    } catch (error) {
      expect(error).not.toBeInstanceOf(NyxAppConnectError);
    }
  });

  it("returns typed errors only after state verification", () => {
    expect(() =>
      parseAppConnectLinkCallback(
        "https://app.example/callback?state=expected&error=access_denied&status=failed",
        "expected",
      ),
    ).toThrow(NyxAppConnectError);
  });

  it("rejects ambiguous or incomplete terminal parameters", () => {
    expect(() =>
      parseAppConnectLinkCallback(`${callback}&status=cancelled`, "expected"),
    ).toThrow("Invalid App Connect Link callback");
    expect(() =>
      parseAppConnectLinkCallback(
        callback.replace(
          "grant_update_required=true",
          "grant_update_required=maybe",
        ),
        "expected",
      ),
    ).toThrow("Invalid App Connect Link callback");
  });
});
