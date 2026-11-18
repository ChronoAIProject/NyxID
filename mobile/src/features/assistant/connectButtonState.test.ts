import assert from "node:assert/strict";
import test from "node:test";
import { connectButtonState } from "./connectButtonState";

const now = Date.parse("2026-11-04T12:00:00Z");
const preview = (status: "pending" | "completed" | "expired" | "cancelled", expires = "2026-11-04T12:14:10Z") =>
  ({
    service_name: "GitHub",
    service_slug: "api-github",
    status,
    expires_at: expires,
  }) as never;

test("a connect button names the service and says how long the link lasts", () => {
  assert.deepEqual(connectButtonState("Connect service", undefined, now), {
    label: "Connect service",
    disabled: false,
    tone: "ready",
  });
  const live = connectButtonState("Connect service", preview("pending"), now);
  assert.equal(live.label, "Connect GitHub");
  assert.equal(live.disabled, false);
  assert.match(live.detail!, /expires in 15 min/);
});

test("used, expired and cancelled links disable the button and say why", () => {
  const done = connectButtonState("Connect service", preview("completed"), now);
  assert.equal(done.label, "GitHub connected");
  assert.equal(done.disabled, true);
  for (const status of ["expired", "cancelled"] as const) {
    const closed = connectButtonState("Connect service", preview(status), now);
    assert.equal(closed.disabled, true, status);
    assert.match(closed.detail!, /Ask NyxBot for a new one/, status);
  }
  const lapsed = connectButtonState(
    "Connect service",
    preview("pending", "2026-11-04T11:59:00Z"),
    now,
  );
  assert.equal(lapsed.disabled, true, "a pending link past its expiry is closed");
  assert.match(lapsed.detail!, /expired/);
});
