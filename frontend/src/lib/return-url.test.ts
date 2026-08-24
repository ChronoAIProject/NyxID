import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resolveTrustedAuthReturnTo } from "./return-url";

describe("resolveTrustedAuthReturnTo", () => {
  beforeEach(() => {
    window.history.pushState(null, "", "/login");
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it.each([
    "//evil.com/x",
    "https://evil.com/",
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "https://[malformed",
  ])("rejects unsafe or malformed return_to %s", (value) => {
    expect(resolveTrustedAuthReturnTo(value)).toBeNull();
  });

  it("accepts same-origin absolute and site-relative paths as normalized hrefs", () => {
    const absolute = `${window.location.origin}/keys?tab=security#sessions`;

    expect(resolveTrustedAuthReturnTo(absolute)).toBe(absolute);
    expect(resolveTrustedAuthReturnTo("/dashboard?tab=home#top")).toBe(
      `${window.location.origin}/dashboard?tab=home#top`,
    );
  });

  it("accepts a configured backend origin", () => {
    vi.stubEnv("VITE_BACKEND_URL", "https://api.example.test/api/v1/");

    expect(
      resolveTrustedAuthReturnTo("https://api.example.test/session?source=device"),
    ).toBe("https://api.example.test/session?source=device");
  });
});
