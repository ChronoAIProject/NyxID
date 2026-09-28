import { beforeEach, describe, expect, it } from "vitest";
import type { KeyInfo, CatalogEntry } from "@/types/keys";
import {
  buildRoutingGroups,
  moveItem,
  orderedIds,
  readPreferences,
  savePreferences,
} from "./service-routing-preview";

function key(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "personal",
    label: "My OpenAI",
    slug: "openai-my-account",
    endpoint_id: "endpoint",
    api_key_id: "credential",
    catalog_service_id: "openai-id",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    is_active: true,
    status: "active",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    auto_connected: false,
    node_id: null,
    node_priority: 0,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-01-01",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    ws_frame_injections: [],
    credential_source: { type: "personal" },
    ...overrides,
  };
}

const org = key({
  id: "org",
  label: "Team OpenAI",
  credential_source: {
    type: "org",
    org_id: "team",
    org_name: "Chrono",
    role: "member",
    allowed: true,
  },
});
const platform = key({
  id: "platform",
  label: "Shared OpenAI",
  auto_connected: true,
});
function group(keys: KeyInfo[]) {
  return buildRoutingGroups(keys, [], [], Date.parse("2026-09-17"))[0]!;
}

beforeEach(() => localStorage.clear());

describe("routing from actual connections", () => {
  it("retains all real records and their provenance without inventing a platform source", () => {
    const result = group([org, key()]);
    expect(result.slug).toBe("openai");
    expect(result.candidates.map((candidate) => candidate.key.id)).toEqual([
      "personal",
      "org",
    ]);
    expect(
      result.candidates.some((candidate) => candidate.tier === "platform"),
    ).toBe(false);
  });

  it("includes a platform source only when a platform service record exists", () => {
    const result = group([key(), platform]);
    expect(
      result.candidates.find((candidate) => candidate.tier === "platform")?.key
        .id,
    ).toBe("platform");
  });

  it("recognizes explicit platform bindings from current main", () => {
    const candidate = group([
      key({
        credential_binding: "platform",
        auto_connected: false,
        api_key_id: null,
        platform_key_available: true,
      }),
    ]).candidates[0]!;
    expect(candidate.tier).toBe("platform");
    expect(candidate.reason).toBe("Not verified");
  });

  it("keeps a configured platform record unavailable when the backend withdraws availability", () => {
    const candidate = group([
      key({ credential_binding: "platform", platform_key_available: false }),
    ]).candidates[0]!;
    expect(candidate.state).toBe("unavailable");
    expect(candidate.reason).toBe("Platform unavailable");
  });

  it("does not mistake an offered platform key for this connection's selected credential", () => {
    const candidate = group([
      key({
        credential_binding: "user",
        auto_connected: true,
        platform_key_available: true,
      }),
    ]).candidates[0]!;
    expect(candidate.tier).toBe("personal");
  });

  it("does not turn catalog entries or shared OAuth apps into connected services", () => {
    const catalog = [
      {
        slug: "openai",
        name: "OpenAI",
        service_type: "http",
        has_platform_oauth_credentials: true,
      },
    ] as CatalogEntry[];
    expect(buildRoutingGroups([], catalog, [], 0)).toEqual([]);
  });

  it("keeps unrelated custom connections separate", () => {
    const result = buildRoutingGroups(
      [
        key(),
        key({
          id: "custom",
          catalog_service_id: null,
          catalog_service_slug: null,
        }),
      ],
      [],
      [],
      0,
    );
    expect(result).toHaveLength(2);
    expect(
      result.find((item) => item.id === "connection:custom")?.canonical,
    ).toBe(false);
  });

  it.each([
    { is_active: false },
    { credential_missing: true },
    { status: "revoked" },
    { status: "failed" },
    { connection_status: "expired" as const },
    { expires_at: "2026-01-01" },
    {
      credential_source: {
        type: "org" as const,
        org_id: "team",
        org_name: "Chrono",
        role: "viewer" as const,
        allowed: false,
      },
    },
  ])(
    "recognizes a known blocker without hiding its repairable card: %j",
    (overrides) => {
      const result = group([key(overrides)]);
      expect(result.candidates).toHaveLength(1);
      expect(result.candidates[0]?.state).toBe("unavailable");
    },
  );

  it.each([
    { connected: true, status: "active" },
    { auto_connected: true },
    { node_id: "node", node_status: "online" },
    { node_id: "node", node_status: "offline" },
    { credential_source: undefined },
    { api_key_id: null },
    { auth_method: "none", api_key_id: null },
    {
      credential_type: "oauth2",
      expires_at: "2026-01-01",
      connection_status: "active" as const,
    },
  ])(
    "does not claim working status from incomplete evidence: %j",
    (overrides) => {
      expect(group([key(overrides)]).candidates[0]?.state).toBe("unverified");
    },
  );

  it("only reorders members of the given pool, removing stale saved IDs", () => {
    const order = orderedIds(["a", "b", "new"], ["b", "foreign", "a", "b"]);
    expect(order).toEqual(["b", "a", "new"]);
    expect(moveItem(order, "foreign", "a")).toEqual(order);
    expect(moveItem(order, "new", "b")).toEqual(["new", "b", "a"]);
  });

  it("scopes pool preferences by account and pool without inheriting old global sorts", () => {
    localStorage.setItem(
      "nyxid-routing-preview-v2:alice",
      JSON.stringify({
        serviceOrder: ["b"],
        connectionOrder: ["b"],
        view: "services",
      }),
    );
    expect(readPreferences("alice")).toEqual({
      view: "connections",
      pools: {},
    });
    savePreferences("alice", {
      view: "services",
      pools: { first: { priority: true, order: ["b", "a"] } },
    });
    expect(readPreferences("alice").pools.first?.order).toEqual(["b", "a"]);
    expect(readPreferences("alice").pools.second).toBeUndefined();
    expect(readPreferences("bob")).toEqual({ view: "connections", pools: {} });
  });
});
