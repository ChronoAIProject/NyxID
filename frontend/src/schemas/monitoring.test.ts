import { expect, it } from "vitest";
import {
  displayedCheckStatus,
  monitoringFormDefaults,
  monitoringFormSchema,
  monitoringPayload,
  type MonitoringCheckResult,
  type MonitoringConfig,
} from "./monitoring";
const config: MonitoringConfig = {
  user_service_id: "service",
  revision: 7,
  owner_config: {
    health: { path: "healthz", expect_status: [200] },
    readiness: null,
  },
};
it("keeps readiness opt-in and serializes only owner assertions", () => {
  const values = monitoringFormDefaults(config);
  expect(values.readiness.enabled).toBe(false);
  expect(monitoringPayload(values)).toEqual(config.owner_config);
  values.health.enabled = false;
  expect(monitoringPayload(values)).toEqual({ health: null, readiness: null });
});
it.each([
  "https://evil.test/",
  "/healthz",
  "healthz?secret=x",
  "healthz#fragment",
  "../health",
  "%252e%252e/health",
  "v1/run",
  "health%3fsecret",
  "health%00",
  "health\\path",
  "health%5cpath",
  "health%zz",
])("rejects unsafe path %s", (path) => {
  const values = monitoringFormDefaults(config);
  values.health.path = path;
  expect(monitoringFormSchema.safeParse(values).success).toBe(false);
});
it("bounds UTF-8 body matching and unique status expectations", () => {
  const values = monitoringFormDefaults(config);
  values.health.body = "好".repeat(342);
  expect(monitoringFormSchema.safeParse(values).success).toBe(false);
  values.health.body = "ok";
  for (const statuses of [
    "200,200",
    "99",
    "600",
    "200,201,202,203,204,205,206,207,208",
  ]) {
    values.health.statuses = statuses;
    expect(monitoringFormSchema.safeParse(values).success).toBe(false);
  }
  values.health.statuses = "200, 204";
  expect(monitoringFormSchema.safeParse(values).success).toBe(true);
});
const now = Date.parse("2026-10-29T12:00:00Z");
const check: MonitoringCheckResult = {
  status: "healthy",
  source: "owner",
  path: "healthz",
  reason: "ok",
  checked_at: new Date(now - 1000).toISOString(),
  stale_at: new Date(now + 1000).toISOString(),
  duration_ms: 2,
  status_code: 200,
  freshness: "fresh",
};
it("expires at the upstream deadline without polling", () => {
  expect(displayedCheckStatus(check, now)).toBe("healthy");
  expect(displayedCheckStatus(check, now + 1000)).toBe("unknown");
});
it.each([
  { freshness: "stale" },
  { freshness: "unknown" },
  { reason: "no_health_path" },
  { checked_at: null },
  { checked_at: "invalid" },
  { checked_at: new Date(now + 6000).toISOString() },
  { stale_at: null },
] as Partial<MonitoringCheckResult>[])(
  "never promotes missing evidence: %j",
  (override) => {
    expect(displayedCheckStatus({ ...check, ...override }, now)).toBe(
      "unknown",
    );
  },
);
