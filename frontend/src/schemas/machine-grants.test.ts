import { describe, expect, it } from "vitest";
import { machineGrantListSchema, machineGrantReadSchema, machineGrantSchema, machineGrantSelectionSchema, readMachineConsentScope } from "./machine-grants";

const nodeId = "b7b0c3da-5d64-4b90-bbae-9bd7aa42104c";
const serviceId = "4b49d5a5-b8d2-4bfc-8290-19baf7a5ce7c";
const assignment = () => ({ capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false }, file_roots: ["/workspace"], service_ids: [] as string[] });
function token(payload: object) { return `header.${btoa(JSON.stringify(payload)).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_")}.signature`; }
describe("Machine grant contracts", () => {
  it("requires a selected real Machine and explicit bounded roots", () => {
    expect(machineGrantSelectionSchema.safeParse({ assignments: {} }).success).toBe(false);
    expect(machineGrantSelectionSchema.safeParse({ assignments: { [nodeId]: assignment() } }).success).toBe(true);
    for (const root of ["workspace", "/workspace/", "/workspace/../etc", "/workspace//tmp", "/workspace/./tmp", "/workspace\\tmp", "/workspace\0tmp"]) {
      expect(machineGrantSelectionSchema.safeParse({ assignments: { [nodeId]: { ...assignment(), file_roots: [root] } } }).success, root).toBe(false);
    }
  });
  it("rejects external browser/computer permissions and service grants without shell", () => {
    for (const capability of ["browser", "computer", "developer_browser"] as const) {
      const row = assignment(); row.capabilities[capability] = true;
      expect(machineGrantSelectionSchema.safeParse({ assignments: { [nodeId]: row } }).success).toBe(false);
    }
    const row = assignment(); row.capabilities.shell = false; row.service_ids = [serviceId];
    expect(machineGrantSelectionSchema.safeParse({ assignments: { [nodeId]: row } }).success).toBe(false);
  });
  it("derives Machine scope display from the protected payload, never a URL label", () => {
    expect(readMachineConsentScope(token({ token_type: "oauth_consent_request", client_id: "client", scope: "openid machine", exp: 1000 }))).toEqual({ scope: "openid machine", clientId: "client", expiresAt: 1000000 });
    expect(readMachineConsentScope("unsigned")).toBeNull();
    expect(readMachineConsentScope(token({ token_type: "access", client_id: "client", scope: "machine", exp: 1000 }))).toBeNull();
  });
  it("parses independent live source and grant validity reasons additively", () => {
    const parsed = machineGrantReadSchema.parse({
      grant: null,
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "expired",
      source_and_grant_valid: false,
    });
    expect(parsed.source_valid).toBe(true);
    expect(parsed.grant_validity_reason).toBe("expired");
    expect(machineGrantReadSchema.safeParse({
      grant: null,
      source_valid: false,
      source_validity_reason: "raw-secret-leak",
      grant_valid: false,
      grant_validity_reason: "revoked",
      source_and_grant_valid: false,
    }).success).toBe(false);
  });
  it("rejects unknown grant expiry instead of presenting it as active", () => {
    const row = { id: "grant", source: { kind: "api_key", api_key_id: "key", api_key_state_version_at_approval: 1 }, revision: 1, policy_revision: 1, status: "active", expires_at: "unknown", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", revoked_at: null, revoked_reason: null, assignments: {} };
    expect(machineGrantSchema.safeParse(row).success).toBe(false);
    expect(machineGrantSchema.safeParse({ ...row, expires_at: "2026-12-01T00:00:00+00:00" }).success).toBe(true);
  });
  it("distinguishes legacy absent pagination from a terminal page and rejects empty cursors", () => {
    expect(machineGrantListSchema.parse({ external_grants: [] }).next_cursor).toBeUndefined();
    expect(machineGrantListSchema.parse({ external_grants: [], next_cursor: null }).next_cursor).toBeNull();
    expect(machineGrantListSchema.safeParse({ external_grants: [], next_cursor: "" }).success).toBe(false);
  });
  it("accepts fully legacy checks but rejects partial or contradictory independent validity", () => {
    const legacy = { grant: null, source_and_grant_valid: true };
    expect(machineGrantReadSchema.safeParse(legacy).success).toBe(true);
    const full = { ...legacy, source_valid: true, source_validity_reason: null, grant_valid: true, grant_validity_reason: null };
    expect(machineGrantReadSchema.safeParse(full).success).toBe(true);
    for (const row of [
      { ...legacy, source_valid: false },
      { ...full, grant_valid: undefined },
      { ...full, source_and_grant_valid: false },
      { ...full, source_validity_reason: "expired" },
      { ...full, grant_valid: false, source_and_grant_valid: false },
    ]) expect(machineGrantReadSchema.safeParse(row).success).toBe(false);
  });
});
