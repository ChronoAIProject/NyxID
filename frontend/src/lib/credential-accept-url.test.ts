import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildStandaloneCredentialAcceptUrl,
  safeRelativeReturnTo,
} from "./credential-accept-url";

const { mockGet } = vi.hoisted(() => ({
  mockGet: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    get: mockGet,
  },
}));

function runtimeConfig(apiBaseUrl = "https://api.example.test/") {
  return {
    api_base_url: apiBaseUrl,
    release_integrity: {
      enabled: true,
      manifest_url: "https://release.example.test/releases.json",
      verification_ttl_secs: 1800,
    },
  };
}

describe("credential accept URL helpers", () => {
  beforeEach(() => {
    mockGet.mockReset();
    window.history.pushState(null, "", "/nodes/current");
  });

  it("builds a backend-origin standalone accept URL and encodes ids", async () => {
    mockGet.mockResolvedValue(runtimeConfig("https://api.example.test/"));

    const href = await buildStandaloneCredentialAcceptUrl(
      "node/with space",
      "pending/with space",
      "/nodes/current?tab=pending#remote",
    );

    expect(mockGet).toHaveBeenCalledWith("/runtime-config");
    const url = new URL(href);
    expect(url.origin).toBe("https://api.example.test");
    expect(url.pathname).toBe(
      "/nodes/node%2Fwith%20space/credentials/pending/pending%2Fwith%20space/accept",
    );
    expect(url.searchParams.get("return_to")).toBe(
      "/nodes/current?tab=pending#remote",
    );
  });

  it("honors same-origin absolute and relative return_to values", () => {
    const sameOrigin = new URL(
      "/orgs/org-1/settings?tab=nodes#pending",
      window.location.origin,
    ).href;

    expect(safeRelativeReturnTo(sameOrigin)).toBe(
      "/orgs/org-1/settings?tab=nodes#pending",
    );
    expect(safeRelativeReturnTo("nodes/node-1?tab=pending")).toBe(
      "/nodes/node-1?tab=pending",
    );
  });

  it("falls back for unsafe return_to values", () => {
    expect(safeRelativeReturnTo("https://evil.example.test/nodes")).toBe(
      "/nodes",
    );
    expect(safeRelativeReturnTo("//evil.example.test/nodes")).toBe("/nodes");
    expect(safeRelativeReturnTo("/\\evil")).toBe("/nodes");
    expect(safeRelativeReturnTo("\\evil")).toBe("/nodes");
  });
});
