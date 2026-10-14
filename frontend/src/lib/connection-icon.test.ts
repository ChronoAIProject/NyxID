import { describe, expect, it } from "vitest";
import { connectionIconUrl } from "./connection-icon";

describe("connectionIconUrl", () => {
  it("prefers the connection's own override", () => {
    expect(
      connectionIconUrl({
        icon_url: "https://example.com/override.svg",
        catalog_icon_url: "https://example.com/catalog.svg",
      }),
    ).toBe("https://example.com/override.svg");
  });

  it("falls back to the catalog icon when there is no override", () => {
    expect(
      connectionIconUrl({ icon_url: null, catalog_icon_url: "https://example.com/catalog.svg" }),
    ).toBe("https://example.com/catalog.svg");
    expect(
      connectionIconUrl({ icon_url: "", catalog_icon_url: "https://example.com/catalog.svg" }),
    ).toBe("https://example.com/catalog.svg");
  });

  it("returns null so ServiceIcon can use the slug glyph", () => {
    expect(connectionIconUrl({})).toBeNull();
    expect(connectionIconUrl(undefined)).toBeNull();
  });
});
