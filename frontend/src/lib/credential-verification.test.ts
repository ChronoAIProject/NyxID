import { describe, expect, it } from "vitest";
import { credentialVerificationMeta, connectionVerificationLabel } from "./credential-verification";
import type { KeyInfo } from "@/types/keys";
import { credentialStatusMeta } from "./credential-status";
import { deriveConnectorItems } from "./assistant/plugins";

const saved = { id: "key", label: "API", credential_type: "api_key", status: "active", is_active: true, node_id: null, last_used_at: "2026-11-11", service_type: "http", catalog_service_slug: null } as KeyInfo;
const evidence = (state: string, reason_code = "provider_accepted") => ({ state, reason_code, observed_at: "2026-11-11", source: "check", can_check: true });
describe("credential verification presentation", () => {
  it("old servers and last_used_at never imply verified", () => {
    expect(credentialVerificationMeta(saved)?.label).toBe("Saved, not verified");
    expect(credentialStatusMeta("active", saved).tooltip).not.toContain("working");
    const card = deriveConnectorItems([saved], []).added[0];
    expect(card?.verificationLabel).toBe("Saved, not verified");
    expect(card?.description).not.toContain("Connected through");
  });
  it.each(["verified", "rejected", "not_verified", "future_state"])("maps %s conservatively", state => {
    expect(credentialVerificationMeta({ ...saved, credential_verification: evidence(state) })?.label)
      .toBe(state === "verified" ? "Verified" : state === "rejected" ? "Rejected by provider" : "Saved, not verified");
  });
  it("403 explains permission denial without calling the key invalid", () => {
    expect(credentialVerificationMeta({ ...saved, credential_verification: evidence("rejected", "provider_forbidden") })?.tooltip).toContain("does not necessarily mean the key is invalid");
  });
  it("unavailable and failed saved credentials never fall back to Connected", () => {
    for (const key of [{ ...saved, credential_missing: true }, { ...saved, status: "failed" }]) {
      const card = deriveConnectorItems([key], []).added[0];
      expect(card?.verificationLabel).toBe(key.credential_missing ? "Unavailable" : "Failed");
      expect(card?.description).not.toContain("Connected through");
    }
  });
  it("preserves OAuth, no-auth, node and platform meaning", () => {
    for (const key of [{ ...saved, credential_type: "oauth2" }, { ...saved, credential_type: "none" }, { ...saved, node_id: "node" }, { ...saved, credential_binding: "platform" as const }, { ...saved, credential_type: "node_managed" }]) {
      expect(credentialVerificationMeta(key)).toBeUndefined();
      expect(credentialStatusMeta("active", key).label).toBe("Active");
    }
  });
  it("a verified connection cannot conceal another unverified or rejected key", () => {
    const verified = { ...saved, credential_verification: evidence("verified") };
    expect(connectionVerificationLabel([verified, saved])).toBe("Saved, not verified");
    expect(connectionVerificationLabel([verified, { ...saved, credential_verification: evidence("rejected") }])).toBe("Rejected by provider");
  });
});
