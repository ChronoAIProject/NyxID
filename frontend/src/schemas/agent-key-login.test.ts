import { describe, expect, it } from "vitest";
import {
  agentKeyApproveSchema,
  agentKeyPreviewSchema,
  effectivePermissions,
  newKeySelection,
} from "./agent-key-login";

describe("Agent Key login schemas", () => {
  it("normalizes codes and defaults new keys to explicit limited access", () => {
    const selection = newKeySelection({
      name: "CLI",
      scopes: ["read", "proxy"],
    });
    expect(selection.success).toBe(true);
    const result = agentKeyApproveSchema.parse({
      user_code: "abcd-efgh",
      selection: selection.data,
    });
    expect(result.user_code).toBe("ABCDEFGH");
    expect(result.selection).toMatchObject({
      kind: "new",
      allow_all_services: false,
      allow_all_nodes: false,
      allowed_service_ids: [],
      allowed_node_ids: [],
    });
  });
  it("rejects conflicting allow-all settings, invalid scope and expired grants", () => {
    for (const input of [
      { allow_all_services: true, allowed_service_ids: ["service"] },
      { allow_all_nodes: true, allowed_node_ids: ["node"] },
      { scopes: "root" },
      { expires_at: "2000-01-01" },
      { rate_limit_per_second: 0 },
    ])
      expect(
        agentKeyApproveSchema.safeParse({
          user_code: "ABCD-EFGH",
          selection: {
            kind: "new",
            name: "CLI",
            scopes: "read proxy",
            ...input,
          },
        }).success,
      ).toBe(false);
  });
  it("parses public preview without credential material", () => {
    const result = agentKeyPreviewSchema.parse({
      initiated_at: "2099-01-01T00:00:00Z",
      expires_at: "2099-01-01T00:10:00Z",
      status: "pending",
      interval: 5,
      requested_profile: "demo",
      api_key: { name: "must-be-dropped" },
      credential: "must-be-dropped",
    });
    expect(result).not.toHaveProperty("credential");
    expect(result).not.toHaveProperty("api_key");
    expect(result.client_ip_attribution).toBe("unavailable");
  });
  it("matches effective backend permissions without treating admin as proxy", () => {
    expect(effectivePermissions("admin")).toBe("read, write");
    expect(effectivePermissions("proxy")).toBe("read, proxy");
    expect(effectivePermissions("services:write")).toBe("read");
  });
});
