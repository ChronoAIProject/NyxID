import { describe, expect, it, vi } from "vitest";
import {
  NyxIDClient,
  NyxAppConnectError,
  type StorageLike,
} from "../src/index.js";

function fixture() {
  const values = new Map<string, string>();
  const storage: StorageLike = {
    getItem: (k) => values.get(k) ?? null,
    setItem: (k, v) => {
      values.set(k, v);
    },
    removeItem: (k) => {
      values.delete(k);
    },
  };
  const fetchFn = vi.fn<typeof fetch>();
  const client = new NyxIDClient({
    baseUrl: "https://id.example",
    clientId: "app",
    redirectUri: "https://app.example/callback",
    storage,
    fetchFn,
  });
  return { client, fetchFn, storage };
}

describe("OAuth app requirements", () => {
  it("encodes repeated resource indicators and keeps PKCE", async () => {
    const { client } = fixture();
    const url = new URL(
      await client.buildAuthorizeUrl({
        state: "correlation",
        resource: [
          "https://id.example/api/v1/proxy/s/github",
          "urn:example:second",
        ],
      }),
    );
    expect(url.searchParams.getAll("resource")).toEqual([
      "https://id.example/api/v1/proxy/s/github",
      "urn:example:second",
    ]);
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("state")).toBe("correlation");
    expect(
      new URL(
        await client.buildAuthorizeUrl({ resource: "urn:single" }),
      ).searchParams.getAll("resource"),
    ).toEqual(["urn:single"]);
  });

  it.each(["", "&state=wrong", "&state=expected&state=expected"])(
    "rejects uncorrelated errors before interpreting them: %s",
    async (state) => {
      const { client, fetchFn } = fixture();
      await client.buildAuthorizeUrl({ state: "expected" });
      let error: unknown;
      try {
        await client.handleRedirectCallback(
          `https://app.example/callback?error=access_denied&error_description=untrusted${state}`,
        );
      } catch (e) {
        error = e;
      }
      expect(error).toBeInstanceOf(Error);
      expect(error).not.toBeInstanceOf(NyxAppConnectError);
      expect((error as Error).message).not.toContain("untrusted");
      expect(fetchFn).not.toHaveBeenCalled();
    },
  );

  it("parses typed app errors after state validation and consumes pending state", async () => {
    const { client, fetchFn, storage } = fixture();
    await client.buildAuthorizeUrl({ state: "expected" });
    await expect(
      client.handleRedirectCallback(
        "https://app.example/callback?state=expected&error=access_denied&nyx_connect_status=cancelled&nyx_connect_reason=user_cancelled&app_connect_link_id=link",
      ),
    ).rejects.toMatchObject({
      name: "NyxAppConnectError",
      error: "access_denied",
      status: "cancelled",
      reason: "user_cancelled",
      appConnectLinkId: "link",
    });
    expect(storage.getItem("nyxid:pending:app")).toBeNull();
    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("preserves granted resources and uses the stored user token for local status", async () => {
    const { client, fetchFn } = fixture();
    await client.buildAuthorizeUrl({
      state: "expected",
      resource: "urn:requested",
    });
    fetchFn.mockResolvedValueOnce(
      Response.json({
        access_token: "user-token",
        token_type: "Bearer",
        expires_in: 3600,
        resource: ["urn:granted"],
      }),
    );
    const tokens = await client.handleRedirectCallback(
      "https://app.example/callback?state=expected&code=code",
    );
    expect(tokens.resource).toEqual(["urn:granted"]);
    const result = {
      requirements_version: 1,
      result_id: "result",
      requirements: [
        {
          requirement_id: "github",
          state: "unsatisfiable",
          reason_code: "slug_shadowed",
          granted_to_caller: false,
        },
      ],
    };
    fetchFn.mockResolvedValueOnce(Response.json(result));
    expect(await client.requirements.status()).toEqual(result);
    expect(fetchFn).toHaveBeenLastCalledWith(
      "https://id.example/api/v1/app-requirements/status",
      { method: "GET", headers: { Authorization: "Bearer user-token" } },
    );
  });
});
