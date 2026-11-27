import { Badge } from "@/components/ui/badge";
import { translateUi } from "@/i18n/use-ui-translation";
import type { MachineGrantRead, OAuthMachineStatus } from "@/schemas/machine-grants";

function effectiveStatus(read: MachineGrantRead, active: boolean): OAuthMachineStatus {
  if (read.oauth_machine_status) {
    return read.oauth_machine_status === "valid" && !active
      ? "grant_expired"
      : read.oauth_machine_status;
  }
  if (read.source_and_grant_valid && active) return "valid";
  // Older APIs can report coarse outcomes without identifying the blocker.
  if (read.source_valid !== true && !read.source_and_grant_valid) return "unknown";
  if (!read.grant) return "grant_missing";
  if (read.grant.status === "revoked") return "grant_revoked";
  if (!active) return "grant_expired";
  return read.grant_validity_reason === "changed" ? "grant_stale" : "unknown";
}

function describe(status: ReturnType<typeof effectiveStatus>): [string, string?] {
  switch (status) {
    case "valid":
      return [translateUi("Machine access granted"), translateUi("New app connections can use this grant after authorizing Machine access.")];
    case "owner_unavailable":
      return [translateUi("Account unavailable"), translateUi("Sign in to an active personal NyxID account to manage Machine access.")];
    case "client_missing":
      return [translateUi("Application unavailable"), translateUi("Ask the app provider whether it still supports connecting to NyxID.")];
    case "client_disabled":
      return [translateUi("Application disabled"), translateUi("Ask the app provider to re-enable its NyxID integration.")];
    case "client_machine_scope_missing":
      return [translateUi("App lacks Machine capability"), translateUi("Ask the app provider to enable Machine access for its NyxID integration.")];
    case "client_proxy_scope_missing":
      return [translateUi("App lacks proxy capability"), translateUi("Ask the app provider to enable proxy access, which is required for Machine permissions.")];
    case "consent_missing":
      return [translateUi("App consent missing"), translateUi("Sign in with NyxID from the app and approve Machine access when prompted.")];
    case "consent_expired":
      return [translateUi("App consent expired"), translateUi("Sign in with NyxID from the app and approve Machine access when prompted.")];
    case "consent_revision_missing":
      return [translateUi("App consent needs renewal"), translateUi("Sign in with NyxID from the app and approve Machine access when prompted.")];
    case "consent_machine_scope_missing":
      return [translateUi("Machine access omitted from consent"), translateUi("Reconnect from the app, choose the Machines to share, and approve the request.")];
    case "consent_proxy_scope_missing":
      return [translateUi("Proxy access omitted from consent"), translateUi("Start a new connection from the app that requests both proxy and Machine access.")];
    case "grant_missing":
      return [translateUi("No Machine grant"), translateUi("Reconnect from the app, choose the Machines to share, and approve the request.")];
    case "grant_stale":
      return [translateUi("Machine grant needs renewal"), translateUi("Reconnect from the app, choose the Machines to share, and approve the request.")];
    case "grant_revoked":
      return [translateUi("Machine grant revoked"), translateUi("Reconnect from the app, choose the Machines to share, and approve the request.")];
    case "grant_expired":
      return [translateUi("Machine grant expired"), translateUi("Reconnect from the app, choose the Machines to share, and approve the request.")];
    case "unknown":
      return [translateUi("Machine access could not be confirmed"), translateUi("Reload permissions. If the status stays unclear, check whether the app offers a NyxID connection with Machine access.")];
  }
}

export function OAuthMachineStatusSummary({ read, active, checking }: {
  readonly read: MachineGrantRead;
  readonly active: boolean;
  readonly checking: boolean;
}) {
  const status = effectiveStatus(read, active);
  const [label, guidance] = checking
    ? [translateUi("Checking Machine permissions…")]
    : describe(status);
  return (
    <div className="space-y-2" role="status">
      <Badge variant={checking ? "secondary" : status === "valid" ? "success" : "warning"}>{label}</Badge>
      {guidance && <p className="text-13 text-muted-foreground">{guidance}</p>}
    </div>
  );
}
