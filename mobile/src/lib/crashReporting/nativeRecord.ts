import {
  validateCrashRecord,
  type CrashReport,
} from "../../../../shared/crash-report";
export type CrashBinding = {
  accountId: string;
  revision: string;
  epochId: string;
};
export type NativeRecord = { binding: CrashBinding; report: CrashReport };
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** Second sanitization boundary. Unknown native fields reject the entire record. */
export function revalidateNativeRecord(raw: unknown): NativeRecord | null {
  if (typeof raw !== "string" || raw.length > 32768) return null;
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object" || Array.isArray(value))
      return null;
    const wrapper = value as Record<string, unknown>;
    if (
      Object.keys(wrapper).sort().join(",") !== "binding,report" ||
      !wrapper.binding ||
      typeof wrapper.binding !== "object" ||
      Array.isArray(wrapper.binding)
    )
      return null;
    const binding = wrapper.binding as Record<string, unknown>;
    if (
      Object.keys(binding).sort().join(",") !== "accountId,epochId,revision" ||
      ![binding.accountId, binding.revision, binding.epochId].every(
        (v) => typeof v === "string" && uuid.test(v),
      )
    )
      return null;
    const report = validateCrashRecord(wrapper.report);
    if (!report || report.platform === "web") return null;
    return {
      binding: {
        accountId: binding.accountId as string,
        revision: binding.revision as string,
        epochId: binding.epochId as string,
      },
      report,
    };
  } catch {
    return null;
  }
}

/** An invalid record grants no capture authority; its UUID only identifies deletion. */
export function nativeRecordEventId(raw: string): string | null {
  if (raw.length > 32768) return null;
  try {
    const value = JSON.parse(raw);
    const id = value?.report?.event_id;
    return typeof id === "string" && uuid.test(id) ? id : null;
  } catch {
    return null;
  }
}
