import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getMockResponse, getMockUser } from "./mock-data";
import { systemSkillsSchema } from "@/schemas/assistant-system-skills";
import { getVisibleAdminNav } from "@/components/dashboard/sidebar";

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
  it("returns a live, richly attributed browser request", () => {
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
      readonly client_city: string;
      readonly client_ip_timezone: string;
      readonly initiating_origin: string;
      readonly initiating_origin_status: string;
      readonly client_timezone: string;
      readonly client_timezone_matches_ip: boolean;
      readonly client_form_factor: string;
      readonly network_relation: string;
      readonly same_ip_as_viewer: boolean;
      readonly initiated_at: string;
      readonly expires_at: string;
      readonly seconds_remaining: number;
      readonly status: string;
    };

    expect(response).toMatchObject({
      client_ip: "103.6.151.42",
      client_ip_attribution: "verified",
      client_country: "SG",
      client_city: "Singapore",
      client_ip_timezone: "Asia/Singapore",
      initiating_origin: "https://login-copy.example",
      initiating_origin_status: "mismatched",
      client_kind: "browser",
      client_app: "Chrome 151.0.7922.174",
      client_platform: "macOS 26.5.2 (arm64)",
      client_timezone: "Europe/Moscow",
      client_timezone_matches_ip: false,
      client_form_factor: "desktop",
      same_ip_as_viewer: false,
      network_relation: "same_network",
      seconds_remaining: 600,
      status: "pending",
    });
    expect(Date.parse(response.initiated_at)).toBe(now.getTime() - 32_000);
    expect(Date.parse(response.expires_at)).toBe(
      now.getTime() + response.seconds_remaining * 1000,
    );
  });
});

describe("built-in skills admin mock", () => {
  beforeEach(() => sessionStorage.clear());

  it("offers the real public skills, rollout flag and admin navigation", () => {
    const data = systemSkillsSchema.parse(getMockResponse("/admin/assistant/system-skills"));
    expect(data.revision).toBe(1);
    expect(data.skills.map(({ skill_id, name, version }) => ({ skill_id, name, version }))).toEqual([
      { skill_id: "b56b5bca-09d6-45f4-991a-05e945ac7eb4", name: "nyxbot", version: "0.1" },
      { skill_id: "9b2b3798-6c7a-4ed9-9c15-43cfb92d342b", name: "nyxbot-ornn-skills", version: "0.1" },
      { skill_id: "959472fc-c4b5-4ede-8d6d-a5e5bf3ec749", name: "nyxbot-cma", version: "0.1" },
    ]);
    expect(getMockUser().capabilities.enabled_features).toContain("assistant:system-skills");
    expect(getVisibleAdminNav(getMockUser())).toEqual(expect.arrayContaining([
      expect.objectContaining({ to: "/admin/assistant/system-skills", label: "Built-in skills" }),
    ]));
  });

  it("saves exact selections in order, rejects stale revisions with 409, and clears", () => {
    const path = "/admin/assistant/system-skills";
    const saved = systemSkillsSchema.parse(getMockResponse(path, "PUT", {
      expected_revision: 1,
      skills: [{ skill: "nyxbot-cma", version: "0.1" }, { skill: "b56b5bca-09d6-45f4-991a-05e945ac7eb4", version: "0.1" }],
    }));
    expect(saved.revision).toBe(2);
    expect(saved.skills.map((skill) => skill.name)).toEqual(["nyxbot-cma", "nyxbot"]);
    expect(saved.updated_by).toBe(getMockUser().id);
    expect(() => getMockResponse(path, "PUT", { expected_revision: 1, skills: [] }))
      .toThrow(expect.objectContaining({ status: 409, errorCode: 1004, message: "System skills changed; reload the current revision" }));
    expect(getMockResponse(path)).toEqual(saved);
    expect(getMockResponse(path, "PUT", { expected_revision: 2, skills: [] })).toMatchObject({ revision: 3, skills: [] });
  });

  it.each([
    [{ skill: "nyxbot", version: "latest" }],
    [{ skill: "unknown", version: "0.1" }],
    [{ skill: "nyxbot", version: "0.2" }],
    [{ skill: "nyxbot", version: "0.1", sha256: "client-chosen" }],
    [{ skill: "nyxbot", version: "0.1" }, { skill: "b56b5bca-09d6-45f4-991a-05e945ac7eb4", version: "0.1" }],
  ])("rejects invalid public skill selections without changing the revision: %j", (...skills) => {
    expect(() => getMockResponse("/admin/assistant/system-skills", "PUT", { expected_revision: 1, skills }))
      .toThrow(expect.objectContaining({ status: 400 }));
    expect(getMockResponse("/admin/assistant/system-skills")).toMatchObject({ revision: 1 });
  });
});
