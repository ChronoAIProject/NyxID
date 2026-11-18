import { translateApp } from "../../i18n/appTranslation";
import {
  canEditConnection,
  type CatalogEntry,
  type ServiceConnection,
} from "./serviceContracts";

export type ConnectionDescription = {
  label: string;
  description: string;
  tone: "success" | "warning" | "error" | "muted";
  recovery?: "continue" | "reconnect";
};

/** Credential health is independent of the reversible service-enabled switch. */
export function describeConnection(
  connection: ServiceConnection,
): ConnectionDescription {
  if (connection.credential_missing) {
    return {
      label: translateApp("Credential missing"),
      description: translateApp(
        "The stored credential is missing. Reconnect or remove this connection.",
      ),
      tone: "error",
      recovery: "reconnect",
    };
  }

  // The credential's lifecycle is authoritative. The separate OAuth
  // connection_status metadata must not hide a failed or revoked credential.
  switch (connection.status) {
    case "pending_auth":
      return {
        label: translateApp("Authorization incomplete"),
        description: translateApp(
          "Authorization has started, but the provider has not returned a credential to NyxID. Continue authentication to finish setup.",
        ),
        tone: "warning",
        recovery: "continue",
      };
    case "failed":
      return {
        label: translateApp("Authorization failed"),
        description: translateApp(
          "Authorization did not complete, or the provider rejected the stored credential. Reconnect to restore access.",
        ),
        tone: "error",
        recovery: "reconnect",
      };
    case "refresh_failed":
      return {
        label: translateApp("Credential renewal failed"),
        description: translateApp(
          "NyxID could not renew this credential automatically. Reconnect to restore access.",
        ),
        tone: "error",
        recovery: "reconnect",
      };
    case "revoked":
      return {
        label: translateApp("Credential revoked"),
        description: translateApp(
          "This credential was revoked and can no longer be used. Add a new connection to authorize access again.",
        ),
        tone: "error",
      };
    case "expired":
      return expiredDescription();
    case "active":
      if (connection.connection_status === "expired") {
        return expiredDescription();
      }
      if (connection.credential_binding === "platform") {
        return {
          label: translateApp("Platform credential"),
          description: translateApp(
            "This connection uses a platform key. Access follows the current platform policy.",
          ),
          tone: "muted",
        };
      }
      if (connection.credential_type === "none") {
        return {
          label: translateApp("No authentication required"),
          description: translateApp(
            "This service does not require a stored credential.",
          ),
          tone: "muted",
        };
      }
      if (connection.credential_type === "node_managed") {
        return {
          label: translateApp("Managed on your node"),
          description: translateApp(
            "Credentials are stored and managed on the connected node. Manage them on that node.",
          ),
          tone: "muted",
        };
      }
      if (isSavedCredential(connection)) {
        const verification = connection.credential_verification;
        if (verification?.state === "verified") return {
          label: translateApp("Verified"), tone: "success",
          description: translateApp(
              "The provider accepted this credential at the last check. Other operations may require more permissions.",
            ),
        };
        if (verification?.state === "rejected") return {
          label: translateApp("Rejected by provider"), tone: "error",
          description: verification.reason_code === "provider_forbidden"
            ? translateApp(
                    "The provider refused this check. Review the credential's permissions; this does not necessarily mean the key is invalid.",
                  )
                : translateApp(
                    "The provider rejected authentication. Check or replace the credential, then check again.",
                  ),
        };
        return { label: translateApp("Saved, not verified"), tone: "warning",
          description: translateApp(
            "NyxID saved this credential. Provider authentication has not been verified.",
          ),
        };
      }
      return {
        label: translateApp("Credential available"),
        description: translateApp(
          "NyxID has an active stored credential for this connection.",
        ),
        tone: "success",
      };
    default:
      return {
        label:
          connection.status.replaceAll("_", " ").trim() ||
          translateApp("Unknown credential status"),
        description: translateApp(
          "NyxID returned an unrecognized credential status. Open this connection in Web for more details.",
        ),
        tone: "warning",
      };
  }
}

function expiredDescription(): ConnectionDescription {
  return {
    label: translateApp("Credential expired"),
    description: translateApp(
      "The stored credential has expired. NyxID renews supported credentials automatically. Check the connection again for an updated status.",
    ),
    tone: "warning",
  };
}

export function isOAuthConnection(
  connection: ServiceConnection,
  entry?: CatalogEntry,
): boolean {
  return (
    connection.credential_type === "oauth2" ||
    entry?.provider_type === "oauth2" ||
    entry?.provider_type === "device_code"
  );
}

/** Mirrors Web's healthy-connection Manage permissions capability gate. */
export function canManageProviderPermissions(
  connection: ServiceConnection,
  entry?: CatalogEntry,
): boolean {
  return (
    Boolean(entry) &&
    canEditConnection(connection) &&
    connection.status === "active" &&
    !connection.credential_missing &&
    connection.credential_binding !== "platform" &&
    entry?.supports_oauth_scopes !== false &&
    entry?.device_code_format !== "openai" &&
    isOAuthConnection(connection, entry)
  );
}

/** Preserve an existing grant during repair rather than adopting new defaults. */
export function reconnectScopeOverride(
  connection: ServiceConnection,
  entry: CatalogEntry,
): string[] | undefined {
  if (
    entry.supports_oauth_scopes === false ||
    entry.device_code_format === "openai" ||
    connection.granted_scopes == null
  ) {
    return undefined;
  }
  // Keep custom scopes even when absent from today's catalog or platform
  // allowlist. Any provider policy rejection must remain visible to the user.
  return [
    ...new Set([
      ...connection.granted_scopes,
      ...(entry.scope_catalog ?? [])
        .filter((scope) => scope.required)
        .map((scope) => scope.scope),
    ]),
  ];
}

export function isSavedCredential(connection: ServiceConnection): boolean {
  return (
    ["api_key", "bearer"].includes(connection.credential_type)
    && connection.credential_binding !== "platform" && !connection.node_id && !connection.has_node_binding
  );
}
