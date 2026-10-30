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
      label: "Credential missing",
      description:
        "The stored credential is missing. Reconnect or remove this connection.",
      tone: "error",
      recovery: "reconnect",
    };
  }

  // The credential's lifecycle is authoritative. The separate OAuth
  // connection_status metadata must not hide a failed or revoked credential.
  switch (connection.status) {
    case "pending_auth":
      return {
        label: "Authorization incomplete",
        description:
          "Authorization has started, but the provider has not returned a credential to NyxID. Continue authentication to finish setup.",
        tone: "warning",
        recovery: "continue",
      };
    case "failed":
      return {
        label: "Authorization failed",
        description:
          "Authorization did not complete, or the provider rejected the stored credential. Reconnect to restore access.",
        tone: "error",
        recovery: "reconnect",
      };
    case "refresh_failed":
      return {
        label: "Credential renewal failed",
        description:
          "NyxID could not renew this credential automatically. Reconnect to restore access.",
        tone: "error",
        recovery: "reconnect",
      };
    case "revoked":
      return {
        label: "Credential revoked",
        description:
          "This credential was revoked and can no longer be used. Add a new connection to authorize access again.",
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
          label: "Platform credential",
          description:
            "This connection uses a platform key. Access follows the current platform policy.",
          tone: "muted",
        };
      }
      if (connection.credential_type === "none") {
        return {
          label: "No authentication required",
          description: "This service does not require a stored credential.",
          tone: "muted",
        };
      }
      if (connection.credential_type === "node_managed") {
        return {
          label: "Managed on your node",
          description:
            "Credentials are stored and managed on the connected node. Manage them on that node.",
          tone: "muted",
        };
      }
      return {
        label: "Credential available",
        description:
          "NyxID has an active stored credential for this connection.",
        tone: "success",
      };
    default:
      return {
        label:
          connection.status.replaceAll("_", " ").trim() ||
          "Unknown credential status",
        description:
          "NyxID returned an unrecognized credential status. Open this connection in Web for more details.",
        tone: "warning",
      };
  }
}

function expiredDescription(): ConnectionDescription {
  return {
    label: "Credential expired",
    description:
      "The stored credential has expired. NyxID renews supported credentials automatically. Check the connection again for an updated status.",
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
