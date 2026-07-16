import { describe, expect, it } from "vitest";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import {
  connectorInitial,
  deriveConnectorItems,
  matchesPluginQuery,
} from "./plugins";

function makeKey(overrides: Partial<KeyInfo>): KeyInfo {
  return {
    id: "key-1",
    label: "OpenAI",
    slug: "openai",
    endpoint_url: "https://api.openai.com/v1",
    credential_type: "api_key",
    catalog_service_slug: "openai",
    service_type: "http",
    ...overrides,
  } as unknown as KeyInfo;
}

function makeEntry(overrides: Partial<CatalogEntry>): CatalogEntry {
  return {
    slug: "openai",
    name: "OpenAI",
    description: "GPT models.",
    base_url: "https://api.openai.com/v1",
    provider_type: null,
    service_type: "http",
    requires_credential: true,
    requires_gateway_url: false,
    token_exchange_credential_fields: null,
    ...overrides,
  } as unknown as CatalogEntry;
}

describe("deriveConnectorItems", () => {
  it("joins on catalog_service_slug, never the user-facing proxy slug", () => {
    const { added, available } = deriveConnectorItems(
      [
        // Proxy slug diverges from the catalog slug (auto-suffixed) — the
        // catalog entry must still count as connected.
        makeKey({ slug: "openai-2", catalog_service_slug: "openai" }),
      ],
      [makeEntry({ slug: "openai" }), makeEntry({ slug: "github", name: "GitHub" })],
    );
    expect(added.map((i) => i.id)).toEqual(["service:openai"]);
    expect(available.map((i) => i.connectSlug)).toEqual(["github"]);
  });

  it("keeps custom keys in Added without hiding catalog entries", () => {
    const { added, available } = deriveConnectorItems(
      [makeKey({ catalog_service_slug: null, label: "Internal API" })],
      [makeEntry({})],
    );
    expect(added).toHaveLength(1);
    expect(added[0]?.manageKeyId).toBe("key-1");
    expect(available).toHaveLength(1);
  });

  it("dedupes multiple connections to one service into a single Added card", () => {
    const { added, available } = deriveConnectorItems(
      [
        makeKey({ id: "key-1", label: "OpenAI" }),
        makeKey({ id: "key-8", label: "Shared GPT-4o" }),
      ],
      [makeEntry({})],
    );
    expect(added).toHaveLength(1);
    expect(added[0]?.meta).toBe("2 connections");
    // Multi-connection cards manage from the keys list, not one key detail.
    expect(added[0]?.manageKeyId).toBeUndefined();
    expect(added[0]?.iconSlug).toBe("openai");
    expect(available).toHaveLength(0);
  });

  it("keeps single-connection manage links pointing at the key detail", () => {
    const { added } = deriveConnectorItems([makeKey({})], [makeEntry({})]);
    expect(added[0]?.manageKeyId).toBe("key-1");
    expect(added[0]?.iconSlug).toBe("openai");
  });

  it("falls back to the catalog description for keys without one", () => {
    const { added } = deriveConnectorItems(
      [makeKey({})],
      [makeEntry({ description: "GPT models." })],
    );
    expect(added[0]?.description).toBe("GPT models.");
  });

  it("derives the auth-kind meta from the catalog entry shape", () => {
    const { available } = deriveConnectorItems(
      [],
      [
        makeEntry({ slug: "a", provider_type: "oauth2" }),
        makeEntry({ slug: "b", provider_type: "device_code" }),
        makeEntry({ slug: "c", requires_credential: false }),
        makeEntry({
          slug: "d",
          token_exchange_credential_fields: [
            { name: "f", label: "F", secret: true },
          ],
        }),
        makeEntry({ slug: "e", requires_gateway_url: true }),
        makeEntry({ slug: "f", service_type: "ssh" }),
        makeEntry({ slug: "g" }),
      ],
    );
    expect(available.map((i) => i.meta)).toEqual([
      "oauth",
      "device code",
      "no credential",
      "token exchange",
      "self-hosted",
      "ssh",
      "api key",
    ]);
    expect(available.map((i) => i.category)).toContain("Connector · SSH");
  });
});

describe("connectorInitial", () => {
  it("uses the first letters of the first two words", () => {
    expect(connectorInitial("OpenClaw Gateway")).toBe("OG");
    expect(connectorInitial("Stripe")).toBe("S");
    expect(connectorInitial("  lark bot  ")).toBe("LB");
    expect(connectorInitial("")).toBe("?");
  });
});

describe("matchesPluginQuery", () => {
  const item = {
    name: "GitHub",
    category: "Connector",
    description: "Repos, issues, and PRs.",
  };

  it("matches case-insensitively across visible card text", () => {
    expect(matchesPluginQuery(item, "github")).toBe(true);
    expect(matchesPluginQuery(item, "ISSUES")).toBe(true);
    expect(matchesPluginQuery(item, "  ")).toBe(true);
    expect(matchesPluginQuery(item, "stripe")).toBe(false);
  });
});
