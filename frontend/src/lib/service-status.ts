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
 */
export type ServiceBadgeVariant =
  | "default"
  | "secondary"
  | "destructive"
  | "outline";

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

export function deriveServiceBadge(
  input: ServiceBadgeInput,
): ServiceBadgeOutput {
  const { isActive, credentialStatus, hasCredential } = input;

  const credentialBlocked =
    hasCredential && credentialStatus !== "" && credentialStatus !== "active";

  if (!isActive) {
    return { variant: "secondary", label: "Inactive", credentialBlocked };
  }
  if (credentialBlocked) {
    return { variant: "outline", label: "Unavailable", credentialBlocked };
  }
  return { variant: "default", label: "Active", credentialBlocked };
}
