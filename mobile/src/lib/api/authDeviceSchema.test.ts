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
      initiating_origin: preview.initiating_origin,
      initiating_origin_status: preview.initiating_origin_status,
      network_relation: preview.network_relation,
      client_timezone: preview.client_timezone,
      client_ip_timezone: preview.client_ip_timezone,
      client_timezone_matches_ip: preview.client_timezone_matches_ip,
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
      initiating_origin: null,
      initiating_origin_status: "absent",
      network_relation: null,
      client_timezone: null,
      client_ip_timezone: null,
      client_timezone_matches_ip: null,
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
    initiating_origin: "https://nyxid.dev",
    initiating_origin_status: "matched",
    network_relation: "same_network",
    client_city: "Singapore",
    client_region: "Singapore",
    client_continent: "AS",
    client_ip_timezone: "Asia/Singapore",
    client_timezone: "Europe/Moscow",
    client_timezone_matches_ip: false,
    client_locale: "en-SG",
    client_form_factor: "desktop",
    client_screen_width: 1512,
    client_screen_height: 982,
    client_device_pixel_ratio: 2,
    client_hardware_concurrency: 12,
    client_device_memory: 16,
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
  assert.equal(preview.initiating_origin_status, "matched");
  assert.equal(preview.network_relation, "same_network");
  assert.equal(preview.client_city, "Singapore");
  assert.equal(preview.client_timezone_matches_ip, false);
  assert.equal(preview.client_form_factor, "desktop");
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
