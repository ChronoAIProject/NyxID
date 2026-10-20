import { describe, expect, it } from "vitest";

import { NyxIDClient } from "../src/index.js";

function client(): NyxIDClient {
  const values = new Map<string, string>();
  return new NyxIDClient({
    baseUrl: "https://nyx.example",
    clientId: "client-1",
    redirectUri: "https://app.example/callback",
    scope: "openid profile email",
    storage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        values.set(key, value);
      },
      removeItem: (key) => {
        values.delete(key);
      },
    },
  });
}

describe("NyxIDClient authorization requests", () => {
  it("requests added scopes and exact services through PKCE", async () => {
    const url = new URL(
      await client().buildAuthorizeUrl({
        includeGrantedScopes: true,
        scope: "account:write",
        requestedServiceIds: ["service-c", "service-d"],
        state: "channel-draft",
      }),
    );

    expect(url.searchParams.get("scope")).toBe("account:write");
    expect(url.searchParams.get("include_granted_scopes")).toBe("true");
    expect(url.searchParams.getAll("requested_service_ids")).toEqual([
      "service-c",
      "service-d",
    ]);
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("state")).toBe("channel-draft");
  });

  it("omits default scopes when only extending service access", async () => {
    const url = new URL(
      await client().buildAuthorizeUrl({
        includeGrantedScopes: true,
        requestedServiceIds: ["service-c"],
      }),
    );
    expect(url.searchParams.has("scope")).toBe(false);
  });

  it("keeps ordinary login defaults and requires explicit incremental mode", async () => {
    const sdk = client();
    const url = new URL(await sdk.buildAuthorizeUrl());
    expect(url.searchParams.get("scope")).toBe("openid profile email");
    expect(url.searchParams.has("include_granted_scopes")).toBe(false);
    await expect(
      sdk.buildAuthorizeUrl({
        requestedServiceIds: ["service-c"],
      }),
    ).rejects.toThrow("requestedServiceIds requires includeGrantedScopes");
  });

  it("validates state before reporting an OAuth denial", async () => {
    const sdk = client();
    const authorize = new URL(
      await sdk.buildAuthorizeUrl({
        includeGrantedScopes: true,
        requestedServiceIds: ["service-c"],
      }),
    );
    const state = authorize.searchParams.get("state")!;
    const callback = new URL("https://app.example/callback");
    callback.searchParams.set("error", "access_denied");
    callback.searchParams.set("state", "forged-state");

    await expect(
      sdk.handleRedirectCallback(callback.toString()),
    ).rejects.toThrow("State mismatch");
    callback.searchParams.set("state", state);
    await expect(
      sdk.handleRedirectCallback(callback.toString()),
    ).rejects.toThrow("OAuth error: access_denied");
    await expect(
      sdk.handleRedirectCallback(callback.toString()),
    ).rejects.toThrow("Missing PKCE state in storage");
  });
});
