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

  it("returns undefined for unknown keys so callers can render a fallback", () => {
    expect(getStatusMeta("node", "does-not-exist")).toBeUndefined();
    expect(getStatusMeta("provider", "")).toBeUndefined();
  });

  it("every registered status has a non-empty label + tooltip", () => {
    const domains: StatusDomain[] = [
      "node",
      "channel_bot",
      "provider",
      "user_service_credential",
    ];
    for (const domain of domains) {
      for (const [key, meta] of Object.entries(STATUS_REGISTRY[domain])) {
        expect(meta.label.trim(), `${domain}.${key} label`).not.toBe("");
        expect(meta.tooltip.trim(), `${domain}.${key} tooltip`).not.toBe("");
      }
    }
  });

  it("remediation links, when present, start with /", () => {
    const domains: StatusDomain[] = [
      "node",
      "channel_bot",
      "provider",
      "user_service_credential",
    ];
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
