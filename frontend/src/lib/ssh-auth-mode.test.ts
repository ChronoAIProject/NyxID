import { describe, expect, it } from "vitest";
import {
  SSH_AUTH_MODE_LABELS,
  getSshAuthModeBadgeVariant,
  getSshAuthModeChangeWarning,
  inferSshAuthMode,
} from "./ssh-auth-mode";

describe("SSH auth mode helpers", () => {
  it("labels node-key mode for the detail badge", () => {
    expect(SSH_AUTH_MODE_LABELS.node_key).toBe("Node Key");
    expect(getSshAuthModeBadgeVariant("node_key")).toBe("success");
    expect(getSshAuthModeBadgeVariant("cert")).toBe("secondary");
  });

  it("falls back from legacy certificate_auth_enabled", () => {
    expect(inferSshAuthMode(undefined, true)).toBe("cert");
    expect(inferSshAuthMode(undefined, false)).toBe("proxy_only");
    expect(inferSshAuthMode("node_key", true)).toBe("node_key");
  });

  it("warns when converting away from node-key mode", () => {
    expect(getSshAuthModeChangeWarning("node_key", "cert")).toContain(
      "nyxid node ssh-credentials prune --stale",
    );
    expect(getSshAuthModeChangeWarning("cert", "node_key")).toBeNull();
    expect(getSshAuthModeChangeWarning("node_key", "node_key")).toBeNull();
  });
});
