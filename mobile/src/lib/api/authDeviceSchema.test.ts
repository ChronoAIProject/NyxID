import assert from "node:assert/strict";
import test from "node:test";

import { parseAuthDevicePreview } from "./authDeviceSchema";

test("normalizes a missing client_ip from an older backend to null", () => {
  const preview = parseAuthDevicePreview({
    client_label: "workstation",
    client_user_agent: "nyxid-cli",
    initiated_at: "2026-08-20T10:00:00Z",
    expires_at: "2026-08-20T10:10:00Z",
    status: "pending",
  });

  assert.equal(preview.client_ip, null);
});
