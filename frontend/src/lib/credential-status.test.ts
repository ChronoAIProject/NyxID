import { describe, expect, it } from "vitest";
import {
  credentialStatusMeta,
  isProblemStatus,
  isReconnectableStatus,
  reconnectLabel,
} from "./credential-status";

describe("credential-status", () => {
  it("treats the three recoverable statuses as reconnectable", () => {
    expect(isReconnectableStatus("failed")).toBe(true);
    expect(isReconnectableStatus("refresh_failed")).toBe(true);
    expect(isReconnectableStatus("pending_auth")).toBe(true);
  });

  it("does not offer reconnect for statuses a re-auth cannot fix", () => {
    // `revoked` is gone for good; `active` has nothing to repair. `expired`
    // is the refresh path's job until it gives up and becomes
    // `refresh_failed`.
    expect(isReconnectableStatus("revoked")).toBe(false);
    expect(isReconnectableStatus("active")).toBe(false);
    expect(isReconnectableStatus("expired")).toBe(false);
  });

  it("says 'Continue authentication' for a flow already in progress", () => {
    expect(reconnectLabel("pending_auth")).toBe("Continue authentication");
    expect(reconnectLabel("failed")).toBe("Reconnect");
    expect(reconnectLabel("refresh_failed")).toBe("Reconnect");
  });

  it("flags every non-active status as needing an explanation", () => {
    expect(isProblemStatus("failed")).toBe(true);
    expect(isProblemStatus("pending_auth")).toBe(true);
    expect(isProblemStatus("active")).toBe(false);
    // A key with no credential at all reports an empty status; there is
    // nothing broken to explain.
    expect(isProblemStatus("")).toBe(false);
  });

  it("explains what 'failed' means rather than echoing the raw status", () => {
    const meta = credentialStatusMeta("failed");
    expect(meta.label).toBe("Failed");
    expect(meta.variant).toBe("destructive");
    expect(meta.tooltip).toMatch(/never completed/i);
    expect(meta.tooltip).toMatch(/rejected/i);
  });

  it("falls back to a readable pill for an unrecognised status", () => {
    const meta = credentialStatusMeta("some_new_backend_state");
    expect(meta.label).toBe("some new backend state");
    expect(meta.variant).toBe("secondary");
    expect(meta.tooltip.trim()).not.toBe("");
  });
});
