import { ApiError } from "../api-error";
import { isAssetLoadError, isAssetRecoveryReloading } from "../chunk-recovery";
import { exceptionClass } from "../../../../shared/crash-report";
export { exceptionClass };
export function excluded(error: unknown): boolean {
  if (error instanceof ApiError && error.status < 500) return true;
  if (
    error instanceof Error &&
    [
      "AbortError",
      "CanceledError",
      "CancelledError",
      "CrashReporterError",
    ].includes(error.name)
  )
    return true;
  return isAssetLoadError(error) && isAssetRecoveryReloading();
}
