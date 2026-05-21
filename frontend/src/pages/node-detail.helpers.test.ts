import { describe, expect, it } from "vitest";
import type { KeyInfo } from "@/types/keys";
import type { NodeAdminInfo, NodeInfo } from "@/types/nodes";
import {
  adminDisplayName,
  canManageNode,
  defaultFieldNameForMethod,
  injectionMethodLabel,
  keyOwnerId,
  nodeOwnerLabel,
} from "./node-detail.helpers";

function makeNode(overrides: Partial<NodeInfo> = {}): NodeInfo {
  return {
    id: "node-1",
    name: "Node One",
    owner: { kind: "user", id: "owner-user", display_name: "Owner User" },
    status: "online",
    is_connected: true,
    last_heartbeat_at: null,
    connected_at: null,
    metadata: null,
    metrics: null,
    binding_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeAdmin(overrides: Partial<NodeAdminInfo> = {}): NodeAdminInfo {
  return {
    user_id: "admin-user",
    display_name: "Admin User",
    email: "admin@example.com",
    role: "admin",
    ...overrides,
  };
}

function makeKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "key-1",
    label: "Key One",
    slug: "key-one",
    endpoint_url: "https://example.com",
    endpoint_id: "endpoint-1",
    credential_type: "api_key",
    auth_method: "header",
    auth_key_name: "X-API-Key",
    status: "active",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    ws_frame_injections: [],
    auto_connected: false,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-01-01T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    ...overrides,
  };
}

describe("nodeOwnerLabel", () => {
  it("returns 'You' when a user-owned node belongs to the current user", () => {
    const owner = { kind: "user", id: "me", display_name: "My Name" } as const;
    expect(nodeOwnerLabel(owner, "me")).toBe("You");
  });

  it("returns the display name when a user-owned node belongs to someone else", () => {
    const owner = {
      kind: "user",
      id: "other",
      display_name: "Other Person",
    } as const;
    expect(nodeOwnerLabel(owner, "me")).toBe("Other Person");
  });

  it("returns the display name for org-owned nodes even if id matches the user", () => {
    // kind is "org", so the user-match branch must not trigger even when ids collide.
    const owner = {
      kind: "org",
      id: "me",
      display_name: "Acme Org",
    } as const;
    expect(nodeOwnerLabel(owner, "me")).toBe("Acme Org");
  });

  it("returns the display name when currentUserId is null", () => {
    const owner = { kind: "user", id: "me", display_name: "My Name" } as const;
    expect(nodeOwnerLabel(owner, null)).toBe("My Name");
  });
});

describe("adminDisplayName", () => {
  it("returns 'You' when the admin is the current user", () => {
    expect(adminDisplayName(makeAdmin({ user_id: "me" }), "me")).toBe("You");
  });

  it("prefers display_name when present", () => {
    const admin = makeAdmin({
      user_id: "other",
      display_name: "Jane Doe",
      email: "jane@example.com",
    });
    expect(adminDisplayName(admin, "me")).toBe("Jane Doe");
  });

  it("falls back to email when display_name is null", () => {
    const admin = makeAdmin({
      user_id: "other",
      display_name: null,
      email: "fallback@example.com",
    });
    expect(adminDisplayName(admin, "me")).toBe("fallback@example.com");
  });

  it("falls back to user_id when both display_name and email are null", () => {
    const admin = makeAdmin({
      user_id: "raw-user-id",
      display_name: null,
      email: null,
    });
    expect(adminDisplayName(admin, "me")).toBe("raw-user-id");
  });
});

describe("canManageNode", () => {
  it("returns false when node is undefined", () => {
    expect(canManageNode(undefined, "me", [makeAdmin({ user_id: "me" })])).toBe(
      false,
    );
  });

  it("returns false when currentUserId is null", () => {
    expect(canManageNode(makeNode(), null, undefined)).toBe(false);
  });

  it("returns true when the user owns a user-owned node", () => {
    const node = makeNode({
      owner: { kind: "user", id: "me", display_name: "Me" },
    });
    expect(canManageNode(node, "me", undefined)).toBe(true);
  });

  it("returns false when a different user owns the user-owned node", () => {
    const node = makeNode({
      owner: { kind: "user", id: "someone-else", display_name: "Else" },
    });
    // Even being listed as admin must not grant manage rights for user-owned nodes.
    expect(canManageNode(node, "me", [makeAdmin({ user_id: "me" })])).toBe(
      false,
    );
  });

  it("returns true for an org-owned node when the user is among its admins", () => {
    const node = makeNode({
      owner: { kind: "org", id: "org-1", display_name: "Acme" },
    });
    const admins = [
      makeAdmin({ user_id: "other-admin" }),
      makeAdmin({ user_id: "me", role: "owner" }),
    ];
    expect(canManageNode(node, "me", admins)).toBe(true);
  });

  it("returns false for an org-owned node when the user is not an admin", () => {
    const node = makeNode({
      owner: { kind: "org", id: "org-1", display_name: "Acme" },
    });
    const admins = [makeAdmin({ user_id: "other-admin" })];
    expect(canManageNode(node, "me", admins)).toBe(false);
  });

  it("returns false for an org-owned node when admins is undefined", () => {
    const node = makeNode({
      owner: { kind: "org", id: "org-1", display_name: "Acme" },
    });
    expect(canManageNode(node, "me", undefined)).toBe(false);
  });

  it("returns false for an org-owned node when admins is empty", () => {
    const node = makeNode({
      owner: { kind: "org", id: "org-1", display_name: "Acme" },
    });
    expect(canManageNode(node, "me", [])).toBe(false);
  });
});

describe("keyOwnerId", () => {
  it("returns the current user id when credential_source is undefined", () => {
    const key = makeKey({ credential_source: undefined });
    expect(keyOwnerId(key, "current-user")).toBe("current-user");
  });

  it("returns the current user id for personal credential sources", () => {
    const key = makeKey({ credential_source: { type: "personal" } });
    expect(keyOwnerId(key, "current-user")).toBe("current-user");
  });

  it("returns null for a personal source when there is no current user", () => {
    const key = makeKey({ credential_source: { type: "personal" } });
    expect(keyOwnerId(key, null)).toBeNull();
  });

  it("returns the org id for org credential sources, ignoring the current user", () => {
    const key = makeKey({
      credential_source: {
        type: "org",
        org_id: "org-42",
        org_name: "Org 42",
        role: "admin",
        allowed: true,
      },
    });
    expect(keyOwnerId(key, "current-user")).toBe("org-42");
  });
});

describe("injectionMethodLabel", () => {
  it("labels the query-param method", () => {
    expect(injectionMethodLabel("query-param")).toBe("Query param");
  });

  it("labels the path-prefix method", () => {
    expect(injectionMethodLabel("path-prefix")).toBe("Path prefix");
  });

  it("labels the header method", () => {
    expect(injectionMethodLabel("header")).toBe("Header");
  });
});

describe("defaultFieldNameForMethod", () => {
  it("defaults query-param to api_key", () => {
    expect(defaultFieldNameForMethod("query-param")).toBe("api_key");
  });

  it("defaults path-prefix to api", () => {
    expect(defaultFieldNameForMethod("path-prefix")).toBe("api");
  });

  it("defaults header to X-API-Key", () => {
    expect(defaultFieldNameForMethod("header")).toBe("X-API-Key");
  });
});
