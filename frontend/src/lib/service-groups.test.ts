import { describe, expect, it } from "vitest";
import { groupServiceConnections } from "./service-groups";
import type { CatalogEntry, KeyInfo } from "@/types/keys";

function key(overrides: Partial<KeyInfo>): KeyInfo {
  return {
    id: "key-1",
    label: "Zoho",
    slug: "api-zoho-mail",
    catalog_service_id: "svc-zoho",
    catalog_service_slug: "api-zoho-mail",
    catalog_service_name: "Zoho Mail",
    ...overrides,
  } as KeyInfo;
}

const catalog = [
  {
    slug: "api-zoho-mail",
    name: "Zoho Mail",
    description: null,
    icon_url: "https://nyx.example/catalog-icons/zoho",
  },
] as unknown as CatalogEntry[];

describe("groupServiceConnections icons", () => {
  it("uses a single connection's override first", () => {
    const [group] = groupServiceConnections(
      [key({ icon_url: "https://example.com/mine.svg" })],
      catalog,
    );
    expect(group?.iconUrl).toBe("https://example.com/mine.svg");
  });

  it("falls back to the connection's catalog icon, then the catalog entry", () => {
    const [fromKey] = groupServiceConnections(
      [key({ catalog_icon_url: "https://nyx.example/catalog-icons/from-key" })],
      catalog,
    );
    expect(fromKey?.iconUrl).toBe("https://nyx.example/catalog-icons/from-key");

    const [fromCatalog] = groupServiceConnections([key({})], catalog);
    expect(fromCatalog?.iconUrl).toBe("https://nyx.example/catalog-icons/zoho");
  });

  it("shows the catalog icon for multi-connection groups instead of one override", () => {
    const [group] = groupServiceConnections(
      [
        key({ id: "a", icon_url: "https://example.com/a.svg" }),
        key({ id: "b", icon_url: "https://example.com/b.svg" }),
      ],
      catalog,
    );
    expect(group?.iconUrl).toBe("https://nyx.example/catalog-icons/zoho");
  });
});
