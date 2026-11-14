import { coarseRuntime } from "./runtime";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import { apiClient } from "../api-client";
import { ApiError } from "../api-error";
import { readAnalyticsConsent } from "../analytics-consent";
import { isDntActive, isSensitivePath } from "../telemetry";
import { parseWebStack } from "./stack";
import { excluded, exceptionClass } from "./classify";
import {
  validateCrashRecord,
  type CrashKind,
  type CrashReport,
} from "../../../../shared/crash-report";
let available = false,
  diagnosticsOffered = false,
  generation = 0,
  stopped = false,
  admissions = 0,
  total = 0;
let permissionRefresh: {
  identity: string;
  epoch: number;
  promise: Promise<void>;
} | undefined;
const seen = new WeakSet<object>();
const signatures = new Map<string, number>();
export function invalidateCrashPermission(): void {
  available = false;
  generation++;
}
export function refreshCrashPermission(offered = diagnosticsOffered): Promise<void> {
  diagnosticsOffered = offered;
  const actor = useAuthStore.getState().user?.id;
  const device = useConsentStore.getState();
  const dnt = isDntActive();
  const identity = JSON.stringify([
    actor,
    device.enabled,
    device.policy,
    device.accountId,
    device.revision,
    offered,
    dnt,
  ]);
  if (
    permissionRefresh?.identity === identity &&
    permissionRefresh.epoch === generation
  )
    return permissionRefresh.promise;
  invalidateCrashPermission();
  const epoch = generation;
  if (
    !offered ||
    !actor ||
    !device.enabled ||
    device.policy !== 2 ||
    device.accountId !== actor ||
    dnt
  )
    return Promise.resolve();
  // Publish the flight before starting I/O so synchronous triggers share it.
  const promise = Promise.resolve()
    .then(async () => {
      if (generation !== epoch) return;
      try {
        const permission = await readAnalyticsConsent(actor);
        if (
          generation === epoch &&
          useConsentStore.getState().revision === device.revision &&
          useAuthStore.getState().user?.id === actor
        ) {
          available =
            permission.diagnostics_status === "available" &&
            permission.enabled &&
            permission.policy_version === "2" &&
            permission.diagnostics_available === true;
          if (available) stopped = false;
        }
      } catch {
        /* Fail closed; capture is optional. */
      }
    })
    .finally(() => {
      if (permissionRefresh?.promise === promise) permissionRefresh = undefined;
    });
  permissionRefresh = { identity, epoch, promise };
  return promise;
}
function localGate(): boolean {
  const device = useConsentStore.getState(),
    actor = useAuthStore.getState().user?.id;
  return (
    available &&
    !stopped &&
    !!actor &&
    device.accountId === actor &&
    device.enabled &&
    device.policy === 2 &&
    !isDntActive() &&
    !isSensitivePath(window.location.pathname)
  );
}
export function reportCrash(
  error: unknown,
  kind: CrashKind = "js_error",
): Promise<void> {
  try {
    return capture(error, kind);
  } catch {
    // Error properties can be accessor functions. Diagnostics must never turn
    // their failure into another global error or change the original fallback.
    return Promise.resolve();
  }
}

function capture(error: unknown, kind: CrashKind): Promise<void> {
  if (!localGate() || excluded(error) || total >= 10 || admissions >= 4)
    return Promise.resolve();
  if (error !== null && typeof error === "object") {
    if (seen.has(error)) return Promise.resolve();
    seen.add(error);
  }
  const actor = useAuthStore.getState().user!.id,
    deviceRevision = useConsentStore.getState().revision,
    epoch = generation;
  const report: CrashReport = {
    schema_version: 1,
    event_id: crypto.randomUUID(),
    captured_at: new Date().toISOString(),
    platform: "web",
    kind,
    exception_class: exceptionClass(error),
    app_version: __APP_VERSION__,
    app_build: __BUILD_ID__,
    artifact_id: __BUILD_ARTIFACT_ID__,
    ...coarseRuntime(navigator.userAgent),
    js_frames: parseWebStack(
      error instanceof Error ? error.stack : "",
      window.location.origin,
    ),
    native_images: [],
    native_frames: [],
  };
  const record = validateCrashRecord(report);
  if (!record) return Promise.resolve();
  const signature = JSON.stringify([
    record.kind,
    record.exception_class,
    record.js_frames,
  ]);
  const now = Date.now();
  if (now - (signatures.get(signature) ?? -Infinity) < 60000)
    return Promise.resolve();
  signatures.set(signature, now);
  admissions++;
  total++;
  const guard = () => {
    if (
      !localGate() ||
      generation !== epoch ||
      useAuthStore.getState().user?.id !== actor ||
      useConsentStore.getState().revision !== deviceRevision
    )
      throw new Error("CrashReporterError");
  };
  return (async () => {
    guard();
    const permission = await readAnalyticsConsent(actor);
    guard();
    if (
      permission.diagnostics_status !== "available" ||
      !permission.enabled ||
      permission.policy_version !== "2" ||
      !permission.diagnostics_available
    ) {
      invalidateCrashPermission();
      return;
    }
    await apiClient("/users/me/crash-reports", {
      method: "POST",
      authorityGuard: guard,
      headers: {
        "X-NyxID-Analytics": "enabled",
        "X-NyxID-Analytics-Account": actor,
      },
      body: {
        account_id: actor,
        revision: permission.revision,
        report: record,
      },
    });
  })()
    .catch((error) => {
      if (
        error instanceof ApiError &&
        [13400, 13401].includes(error.errorCode)
      ) {
        stopped = true;
        invalidateCrashPermission();
      }
    })
    .finally(() => {
      admissions--;
    });
}
