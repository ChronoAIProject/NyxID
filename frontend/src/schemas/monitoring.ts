import { z } from "zod";

export interface MonitoringCheck {
  path: string;
  expect_status: number[];
  body_contains?: string;
}

export interface MonitoringConfig {
  user_service_id: string;
  revision: number;
  owner_config: {
    health: MonitoringCheck | null;
    readiness: MonitoringCheck | null;
  };
}

export interface MonitoringCheckResult {
  status:
    | "healthy"
    | "unhealthy"
    | "ready"
    | "not_ready"
    | "not_configured"
    | "unknown";
  source: "owner" | "builtin" | "discovered" | "none";
  path: string;
  reason: string;
  checked_at: string | null;
  duration_ms: number | null;
  status_code: number | null;
  freshness: "fresh" | "stale" | "unknown";
  interval_ms?: number | null;
  stale_at: string | null;
}

export interface MonitoringResult {
  service: { id: string };
  health: MonitoringCheckResult;
  readiness: MonitoringCheckResult | null;
}

export interface MonitoringResponse {
  service_id: string;
  can_manage: boolean;
  can_probe: boolean;
  status: MonitoringResult;
  configuration: MonitoringConfig | null;
}

const bytes = (value: string) => new TextEncoder().encode(value).length;

function safeRelativePath(path: string): boolean {
  if (
    !path ||
    bytes(path) > 512 ||
    path.startsWith("/") ||
    /[:\\?#\s]/.test(path) ||
    [...path].some(
      (char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
    )
  )
    return false;
  try {
    let decoded = path;
    for (let i = 0; i < 4; i++) {
      const next = decodeURIComponent(decoded);
      if (next === decoded) break;
      decoded = next;
    }
    return (
      !/[\\?#]/.test(decoded) &&
      ![...decoded].some(
        (char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
      ) &&
      !decoded
        .split("/")
        .some((segment) =>
          [
            ".",
            "..",
            "create",
            "execute",
            "generate",
            "run",
            "submit",
          ].includes(segment.toLowerCase()),
        )
    );
  } catch {
    return false;
  }
}

function statusList(value: string): number[] {
  return value.split(",").map((part) => Number(part.trim()));
}

const checkFields = z
  .object({
    enabled: z.boolean(),
    path: z.string(),
    statuses: z.string(),
    body: z.string(),
  })
  .superRefine((value, context) => {
    if (!value.enabled) return;
    if (!safeRelativePath(value.path.trim()))
      context.addIssue({
        code: "custom",
        path: ["path"],
        message:
          "Use a safe relative GET path, such as healthz. URLs, queries and operation paths are not allowed.",
      });
    const statuses = statusList(value.statuses);
    if (
      !/^\s*\d{3}(\s*,\s*\d{3})*\s*$/.test(value.statuses) ||
      statuses.length > 8 ||
      new Set(statuses).size !== statuses.length ||
      statuses.some((status) => status < 100 || status > 599)
    )
      context.addIssue({
        code: "custom",
        path: ["statuses"],
        message:
          "Enter up to 8 unique HTTP status codes (100–599), separated by commas.",
      });
    if (bytes(value.body) > 1024)
      context.addIssue({
        code: "custom",
        path: ["body"],
        message: "Body expectation must be at most 1,024 bytes.",
      });
  });

export const monitoringFormSchema = z.object({
  health: checkFields,
  readiness: checkFields,
});
export type MonitoringFormValues = z.infer<typeof monitoringFormSchema>;

export function monitoringFormDefaults(
  config: MonitoringConfig,
): MonitoringFormValues {
  const fields = (check: MonitoringCheck | null) => ({
    enabled: Boolean(check),
    path: check?.path ?? "",
    statuses: (check?.expect_status ?? [200]).join(", "),
    body: check?.body_contains ?? "",
  });
  return {
    health: fields(config.owner_config.health),
    readiness: fields(config.owner_config.readiness),
  };
}

export function monitoringPayload(
  values: MonitoringFormValues,
): MonitoringConfig["owner_config"] {
  const check = (
    value: MonitoringFormValues["health"],
  ): MonitoringCheck | null =>
    value.enabled
      ? {
          path: value.path.trim(),
          expect_status: statusList(value.statuses),
          ...(value.body ? { body_contains: value.body } : {}),
        }
      : null;
  return { health: check(values.health), readiness: check(values.readiness) };
}

/** Status is an observation with an expiry, never a permanent green badge. */
export function displayedCheckStatus(
  check: MonitoringCheckResult,
  now = Date.now(),
): MonitoringCheckResult["status"] {
  if (check.status === "not_configured") return "not_configured";
  const at = check.checked_at ? Date.parse(check.checked_at) : NaN;
  const staleAt = check.stale_at ? Date.parse(check.stale_at) : NaN;
  if (
    check.freshness !== "fresh" ||
    !Number.isFinite(at) ||
    at > now + 5000 ||
    !Number.isFinite(staleAt) ||
    now >= staleAt ||
    check.reason === "no_health_path"
  )
    return "unknown";
  return check.status;
}
