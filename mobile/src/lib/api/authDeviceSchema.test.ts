import assert from "node:assert/strict";
import test from "node:test";

import { parseAuthDevicePreview } from "./authDeviceSchema";

test("normalizes additive fields missing from an older backend", () => {
  const preview = parseAuthDevicePreview({
    client_label: "workstation",
    client_user_agent: "nyxid-cli",
    initiated_at: "2026-08-20T10:00:00Z",
    expires_at: "2026-08-20T10:10:00Z",
    status: "pending",
  });

  assert.deepEqual(
    {
      client_ip: preview.client_ip,
      client_ip_attribution: preview.client_ip_attribution,
      client_country: preview.client_country,
      client_kind: preview.client_kind,
      client_app: preview.client_app,
      client_platform: preview.client_platform,
      same_ip_as_viewer: preview.same_ip_as_viewer,
      seconds_remaining: preview.seconds_remaining,
    },
    {
      client_ip: null,
      client_ip_attribution: "unavailable",
      client_country: null,
      client_kind: "unknown",
      client_app: null,
      client_platform: null,
      same_ip_as_viewer: null,
      seconds_remaining: null,
    },
  );
});

test("accepts verbose requester attribution", () => {
  const preview = parseAuthDevicePreview({
    client_label: "workstation",
    client_user_agent: "nyxid-cli/1.4.2 (macos; aarch64)",
    client_ip: "203.0.113.10",
    client_ip_attribution: "verified",
    client_country: "SG",
    client_kind: "cli",
    client_app: "NyxID CLI 1.4.2",
    client_platform: "macOS (aarch64)",
    same_ip_as_viewer: false,
    seconds_remaining: 583,
    initiated_at: "2026-08-20T10:00:00Z",
    expires_at: "2026-08-20T10:10:00Z",
    status: "pending",
  });

  assert.equal(preview.client_country, "SG");
  assert.equal(preview.client_ip_attribution, "verified");
  assert.equal(preview.client_kind, "cli");
  assert.equal(preview.client_app, "NyxID CLI 1.4.2");
  assert.equal(preview.client_platform, "macOS (aarch64)");
  assert.equal(preview.same_ip_as_viewer, false);
  assert.equal(preview.seconds_remaining, 583);
});

test("bounds and strips controls from requester display strings", () => {
  const preview = parseAuthDevicePreview({
    client_label: `host\0${"x".repeat(100)}`,
    client_user_agent: `agent\n${"y".repeat(300)}`,
    initiated_at: "2026-08-20T10:00:00Z",
    expires_at: "2026-08-20T10:10:00Z",
    status: "pending",
  });

  assert.equal(preview.client_label?.length, 64);
  assert.equal(preview.client_user_agent?.length, 256);
  assert.equal(preview.client_label?.includes("\0"), false);
  assert.equal(preview.client_user_agent?.includes("\n"), false);
});
