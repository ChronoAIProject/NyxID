import { describe, expect, it } from "vitest";
import {
  STATUS_REGISTRY,
  getStatusMeta,
  type StatusDomain,
} from "./status-contract";

describe("status-contract", () => {
  it("returns the right meta for known (domain, key) pairs", () => {
    expect(getStatusMeta("node", "Online")?.label).toBe("Online");
    expect(getStatusMeta("node", "Online")?.variant).toBe("success");
    expect(getStatusMeta("provider", "refresh_failed")?.label).toBe(
      "Refresh Failed",
    );
    expect(getStatusMeta("channel_bot", "pending_webhook")?.variant).toBe(
      "warning",
    );
  });

  it("covers every UserApiKey.status the backend writes", () => {
    // `backend/src/models/user_api_key.rs` — an unregistered status would
    // fall back to a bare pill with no explanation, which is what the
    // credential domain exists to prevent.
    for (const status of [
      "active",
      "pending_auth",
      "expired",
      "revoked",
      "failed",
      "refresh_failed",
    ]) {
      expect(getStatusMeta("credential", status), status).toBeDefined();
    }
  });

  it("returns undefined for unknown keys so callers can render a fallback", () => {
    expect(getStatusMeta("node", "does-not-exist")).toBeUndefined();
    expect(getStatusMeta("provider", "")).toBeUndefined();
  });

  it("every registered status has a non-empty label + tooltip", () => {
    // Derived from the registry rather than hand-listed, so a newly added
    // domain is covered without remembering to extend this array.
    const domains = Object.keys(STATUS_REGISTRY) as StatusDomain[];
    for (const domain of domains) {
      for (const [key, meta] of Object.entries(STATUS_REGISTRY[domain])) {
        expect(meta.label.trim(), `${domain}.${key} label`).not.toBe("");
        expect(meta.tooltip.trim(), `${domain}.${key} tooltip`).not.toBe("");
      }
    }
  });

  it("remediation links, when present, start with /", () => {
    // Derived from the registry rather than hand-listed, so a newly added
    // domain is covered without remembering to extend this array.
    const domains = Object.keys(STATUS_REGISTRY) as StatusDomain[];
    for (const domain of domains) {
      for (const [key, meta] of Object.entries(STATUS_REGISTRY[domain])) {
        if (meta.remediation) {
          expect(
            meta.remediation.href.startsWith("/"),
            `${domain}.${key} remediation href`,
          ).toBe(true);
          expect(meta.remediation.label.trim()).not.toBe("");
        }
      }
    }
  });
});
