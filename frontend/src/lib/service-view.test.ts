import { describe, expect, it } from "vitest";
import { matchingConnections } from "./service-view";
import {
  DEFAULT_SERVICE_FILTERS,
  serviceViewSchema,
  sameServiceFilters,
} from "@/schemas/service-view";
import type { ServiceConnectionGroup } from "./service-groups";
import type { KeyInfo } from "@/types/keys";

const group: ServiceConnectionGroup = {
  id: "catalog:openai",
  name: "OpenAI",
  slug: "openai",
  iconSlug: "openai",
  iconUrl: null,
  description: null,
  connections: [
    {
      id: "personal",
      label: "Personal development",
      slug: "openai-personal",
      credential_source: { type: "personal" },
      is_active: true,
      status: "expired",
      service_type: "http",
      auto_connected: false,
    },
    {
      id: "org",
      label: "Team",
      slug: "openai-team",
      credential_source: { type: "org", org_id: "org-1", org_name: "Chrono" },
      is_active: false,
      status: "active",
      service_type: "http",
      auto_connected: false,
    },
    {
      id: "platform",
      label: "Platform",
      slug: "openai",
      credential_binding: "platform",
      is_active: true,
      service_type: "http",
      auto_connected: true,
    },
  ] as KeyInfo[],
};

describe("service view matching", () => {
  it("shows only personal connections by default", () => {
    expect(
      matchingConnections(group, DEFAULT_SERVICE_FILTERS).map((key) => key.id),
    ).toEqual(["personal"]);
  });
  it("combines filters on the same connection and uses service state, not credential status", () => {
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "org",
        state: "enabled",
      }),
    ).toEqual([]);
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        state: "disabled",
      }).map((key) => key.id),
    ).toEqual(["org"]);
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "personal",
        state: "enabled",
      }).map((key) => key.id),
    ).toEqual(["personal"]);
  });
  it("searches both service and connection identity without modifying the group", () => {
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        search: " Chrono ",
      }).map((key) => key.id),
    ).toEqual(["org"]);
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        search: "OPENAI",
      }),
    ).toHaveLength(3);
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        service_type: "ssh",
      }),
    ).toEqual([]);
    expect(
      matchingConnections(group, {
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        show_auto_connected: false,
      }),
    ).toHaveLength(2);
    expect(group.connections).toHaveLength(3);
  });
});

describe("saved service selections", () => {
  it("migrates legacy single selections and compares unordered sets", () => {
    const legacy = {
      search: "",
      source: "all",
      state: "all",
      service_type: "all",
      show_auto_connected: true,
    };
    expect(
      serviceViewSchema.parse({
        ...legacy,
        organization_id: "org-1",
        service_group_id: "catalog:openai",
      }),
    ).toEqual({
      ...DEFAULT_SERVICE_FILTERS,
      source: "all",
      organization_ids: ["org-1"],
      service_group_ids: ["catalog:openai"],
    });
    expect(serviceViewSchema.parse(legacy)).toEqual({
      ...DEFAULT_SERVICE_FILTERS,
      source: "all",
    });
    expect(
      serviceViewSchema.parse({
        ...legacy,
        organization_id: null,
        service_group_id: null,
      }),
    ).toEqual({ ...DEFAULT_SERVICE_FILTERS, source: "all" });
    expect(
      sameServiceFilters(
        {
          ...DEFAULT_SERVICE_FILTERS,
          source: "all",
          organization_ids: ["one", "two"],
        },
        {
          ...DEFAULT_SERVICE_FILTERS,
          source: "all",
          organization_ids: ["two", "one"],
        },
      ),
    ).toBe(true);
    expect(
      serviceViewSchema.safeParse({
        ...DEFAULT_SERVICE_FILTERS,
        source: "all",
        organization_ids: Array(101).fill("org"),
      }).success,
    ).toBe(false);
  });
  it("matches any selected organization and service, requiring both filter groups", () => {
    const filters = {
      ...DEFAULT_SERVICE_FILTERS,
      source: "all" as const,
      organization_ids: ["org-1", "org-2"],
      service_group_ids: ["catalog:openai", "catalog:codex"],
    };
    expect(matchingConnections(group, filters).map((key) => key.id)).toEqual([
      "org",
    ]);
    expect(
      matchingConnections(group, {
        ...filters,
        service_group_ids: ["catalog:codex"],
      }),
    ).toEqual([]);
    expect(
      matchingConnections(group, { ...filters, organization_ids: ["org-2"] }),
    ).toEqual([]);
  });
});
