import { afterEach, describe, expect, it, vi } from "vitest";
import { getMockResponse } from "./mock-data";

afterEach(() => {
  vi.useRealTimers();
});

function catalogSlugs(endpoint: string): string[] {
  const response = getMockResponse(endpoint) as {
    readonly entries: readonly { readonly slug: string }[];
  };
  return response.entries.map((entry) => entry.slug);
}

describe("mock catalog", () => {
  it("exposes the action contract slug only through the full catalog", () => {
    expect(catalogSlugs("/catalog")).not.toContain("api-github");
    expect(catalogSlugs("/catalog?include_all=true")).toContain("api-github");
  });
});

describe("auth device preview mock", () => {
  it("returns a live, fully attributed CLI request", () => {
    vi.useFakeTimers();
    const now = new Date("2026-08-25T12:00:00Z");
    vi.setSystemTime(now);

    const response = getMockResponse(
      "/auth/device/preview",
      "POST",
    ) as {
      readonly client_ip: string;
      readonly client_ip_attribution: string;
      readonly client_country: string;
      readonly client_kind: string;
      readonly client_app: string;
      readonly client_platform: string;
      readonly client_user_agent: string;
      readonly same_ip_as_viewer: boolean;
      readonly initiated_at: string;
      readonly expires_at: string;
      readonly seconds_remaining: number;
      readonly status: string;
    };

    expect(response).toMatchObject({
      client_ip: "8.8.8.8",
      client_ip_attribution: "verified",
      client_country: "US",
      client_kind: "cli",
      client_app: "NyxID CLI 1.4.2",
      client_platform: "macOS (aarch64)",
      client_user_agent: "nyxid-cli/1.4.2 (macos; aarch64)",
      same_ip_as_viewer: false,
      seconds_remaining: 600,
      status: "pending",
    });
    expect(Date.parse(response.initiated_at)).toBe(now.getTime() - 32_000);
    expect(Date.parse(response.expires_at)).toBe(
      now.getTime() + response.seconds_remaining * 1000,
    );
  });
});
