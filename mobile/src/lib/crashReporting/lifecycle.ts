import type { ConsentSnapshot } from "../analyticsConsentState";
import type { AnalyticsPermission, RelayRequest } from "../analyticsRelay";
import {
  nativeRecordEventId,
  revalidateNativeRecord,
  type CrashBinding,
} from "./nativeRecord";
import {
  createCrashRelay,
  crashAuthorityError,
  excludedMobileError,
  permissionDecision,
} from "./reporter";
import {
  exceptionClass,
  validateCrashRecord,
  type CrashKind,
  type CrashReport,
} from "../../../../shared/crash-report";
import { parseMobileStack } from "./stack";
export interface CrashNativeModule {
  getAppIdentity(): { version: string; build: string; osVersion?: string };
  getBinding(): CrashBinding | null;
  arm(binding: CrashBinding): boolean;
  pause(): void;
  disarm(): void;
  recordFatalJs(json: string): void;
  collectPending(): string[];
  removeRecords(ids: string[]): void;
}
export function createCrashLifecycle(deps: {
  native: CrashNativeModule;
  snapshot: () => ConsentSnapshot;
  request: RelayRequest;
  uuid: () => string;
  platform: "ios" | "android";
  osVersion: () => string;
  artifactId?: string;
}) {
  let binding: CrashBinding | null = null,
    generation = 0,
    ready = false,
    restoring = true;
  const seen = new WeakSet<object>();
  let count = 0;
  const eligible = () => {
    const s = deps.snapshot();
    return (
      !restoring && s.isHydrated && s.enabled && s.policy === 2 && !!s.accountId
    );
  };
  const disarm = () => {
    generation++;
    binding = null;
    ready = false;
    deps.native.disarm();
  };
  const pause = () => {
    generation++;
    ready = false;
    deps.native.pause();
  };
  const current = (b: CrashBinding) =>
    eligible() &&
    ready &&
    binding?.epochId === b.epochId &&
    binding.revision === b.revision &&
    deps.snapshot().accountId === b.accountId;
  const send = createCrashRelay(
    deps.request,
    current,
    disarm,
    () => generation,
    pause,
  );
  const makeRecord = (error: unknown, kind: CrashKind): CrashReport | null => {
    if (
      !binding ||
      !current(binding) ||
      count >= 10 ||
      excludedMobileError(error)
    )
      return null;
    if (
      error instanceof Error &&
      ["AbortError", "CanceledError", "CancelledError"].includes(error.name)
    )
      return null;
    if (error && typeof error === "object") {
      if (seen.has(error)) return null;
      seen.add(error);
    }
    const identity = deps.native.getAppIdentity();
    const record = validateCrashRecord({
      schema_version: 1,
      event_id: deps.uuid(),
      captured_at: new Date().toISOString(),
      platform: deps.platform,
      kind,
      exception_class: exceptionClass(error),
      app_version: identity.version,
      app_build: identity.build,
      ...(deps.artifactId ? { artifact_id: deps.artifactId } : {}),
      os_version: deps.osVersion(),
      js_frames: parseMobileStack(error instanceof Error ? error.stack : ""),
      native_images: [],
      native_frames: [],
    });
    if (record) count++;
    return record;
  };
  return {
    disarm,
    setRestoring(value: boolean) {
      restoring = value;
      if (value) {
        generation++;
        ready = false;
        deps.native.pause();
      } else return this.refresh();
    },
    async refresh() {
      generation++;
      ready = false;
      deps.native.pause();
      // A restore timeout can end the splash before an account is known.
      // Only an explicit sign-out or a known successor authorizes a purge.
      if (restoring || !deps.snapshot().isHydrated || !deps.snapshot().accountId)
        return;
      if (!eligible()) {
        disarm();
        return;
      }
      const start = deps.snapshot(),
        epoch = generation;
      // Invalidate a known predecessor before awaiting any network operation.
      const stored = deps.native.getBinding();
      if (stored && stored.accountId !== start.accountId) deps.native.disarm();
      const guard = () => {
        const now = deps.snapshot();
        if (
          epoch !== generation ||
          now.revision !== start.revision ||
          now.accountId !== start.accountId ||
          !eligible()
        )
          throw crashAuthorityError();
      };
      try {
        const permission = await deps.request<AnalyticsPermission>(
          "/users/me/preferences/analytics",
          { expectedAccountId: start.accountId!, authorityGuard: guard },
        );
        guard();
        const decision = permissionDecision(permission, start.accountId!);
        if (decision !== "available") {
          if (decision === "disarm") disarm();
          else pause();
          return;
        }
        const previous = deps.native.getBinding();
        if (
          previous?.accountId === start.accountId &&
          previous.revision === permission.revision
        )
          binding = previous;
        else {
          deps.native.disarm();
          binding = {
            accountId: start.accountId!,
            revision: permission.revision,
            epochId: deps.uuid(),
          };
        }
        if (!deps.native.arm(binding)) {
          disarm();
          return;
        }
        ready = true;
        for (const raw of deps.native.collectPending()) {
          guard();
          const record = revalidateNativeRecord(raw);
          if (
            !record ||
            record.report.platform !== deps.platform ||
            !binding ||
            record.binding.epochId !== binding.epochId ||
            record.binding.accountId !== binding.accountId ||
            record.binding.revision !== binding.revision
          ) {
            const eventId = nativeRecordEventId(raw);
            if (eventId) deps.native.removeRecords([eventId]);
            continue;
          }
          if (await send(record.report, binding))
            deps.native.removeRecords([record.report.event_id]);
        }
      } catch (error) {
        if (epoch === generation) {
          const code =
            error && typeof error === "object"
              ? (error as { errorCode?: number }).errorCode
              : undefined;
          const snapshot = deps.snapshot();
          if (
            code === 13400 ||
            (snapshot.accountId && (!snapshot.enabled || snapshot.policy !== 2))
          )
            disarm();
          else pause();
        }
      }
    },
    fatal(error: unknown) {
      try {
        const record = makeRecord(error, "js_fatal");
        if (record) deps.native.recordFatalJs(JSON.stringify(record));
      } catch {
        /* Never interfere with RN's fatal handler. */
      }
    },
    async capture(error: unknown, kind: CrashKind = "js_error") {
      try {
        const record = makeRecord(error, kind);
        const b = binding;
        if (record && b) await send(record, b);
      } catch {
        /* Optional diagnostics fail closed. */
      }
    },
  };
}
