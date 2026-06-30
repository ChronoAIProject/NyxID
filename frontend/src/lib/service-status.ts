/**
 * Effective-availability derivation for the Service section of the key detail
 * page (see NyxID#329).
 *
 * The underlying MongoDB record stores two independent pieces of state:
 *
 * - `UserService.is_active` — a user-controlled boolean (Activate / Deactivate
 *   toggle on the service record).
 * - `UserApiKey.status` — the credential lifecycle, one of `"active"`,
 *   `"pending_auth"`, `"expired"`, `"revoked"`, `"failed"`,
 *   `"refresh_failed"`.
 *
 * The Service badge used to display `is_active ? "Active" : "Inactive"` only,
 * which caused a misleading "Active" state after switching routing from
 * `Route via Node` back to `Direct` when no direct credential existed: the
 * service record stayed enabled but its credential became `pending_auth`, so
 * real requests failed with `1000 - API key is pending_auth`.
 *
 * This helper returns the composed badge state so the detail page matches the
 * availability truth the proxy will act on.
 *
 * Wave B B.4 deferred migration (landed in Wave C C.1): the (label, variant)
 * for each output state now resolves through
 * `STATUS_REGISTRY.user_service_credential` in `@/lib/status-contract`, so
 * changing the user-facing copy is a one-PR edit on the registry. The
 * derivation logic (when to surface "Unavailable" vs "Active") stays here
 * because it composes two backend signals — that's not a registry concern.
 */
import {
  getStatusMeta,
  type StatusVariant,
} from "@/lib/status-contract";

export type ServiceBadgeVariant = Extract<
  StatusVariant,
  "success" | "secondary" | "destructive"
>;

export interface ServiceBadgeInput {
  readonly isActive: boolean;
  /** API key status. Empty string is treated the same as "no credential". */
  readonly credentialStatus: string;
  /**
   * Whether this service has an associated credential. Services without a
   * credential (auto-connected, no-auth downstreams) skip the credential-
   * readiness check entirely.
   */
  readonly hasCredential: boolean;
}

export interface ServiceBadgeOutput {
  readonly variant: ServiceBadgeVariant;
  readonly label: string;
  /**
   * True when the service record is enabled but its credential is not in an
   * `"active"` state. Callers use this to render an inline explanation under
   * the badge.
   */
  readonly credentialBlocked: boolean;
}

function resolve(key: "active" | "inactive" | "unavailable"): {
  readonly variant: ServiceBadgeVariant;
  readonly label: string;
} {
  const meta = getStatusMeta("user_service_credential", key);
  // The registry seeds these three keys, so the fallback only fires if
  // someone accidentally drops them. Keep narrow fallback variants so the
  // type contract stays tight — secondary is the safe neutral default.
  return {
    variant: (meta?.variant as ServiceBadgeVariant | undefined) ?? "secondary",
    label: meta?.label ?? key,
  };
}

export function deriveServiceBadge(
  input: ServiceBadgeInput,
): ServiceBadgeOutput {
  const { isActive, credentialStatus, hasCredential } = input;

  const credentialBlocked =
    hasCredential && credentialStatus !== "" && credentialStatus !== "active";

  if (!isActive) {
    return { ...resolve("inactive"), credentialBlocked };
  }
  if (credentialBlocked) {
    return { ...resolve("unavailable"), credentialBlocked };
  }
  return { ...resolve("active"), credentialBlocked };
}
