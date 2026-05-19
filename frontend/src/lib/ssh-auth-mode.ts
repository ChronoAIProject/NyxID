import type { SshAuthMode } from "@/schemas/services";

export const SSH_AUTH_MODE_LABELS: Readonly<Record<SshAuthMode, string>> = {
  cert: "Cert",
  node_key: "Node Key",
  proxy_only: "Proxy Only",
};

export function inferSshAuthMode(
  mode: SshAuthMode | null | undefined,
  certificateAuthEnabled: boolean | null | undefined,
): SshAuthMode {
  return mode ?? (certificateAuthEnabled ? "cert" : "proxy_only");
}

export function getSshAuthModeBadgeVariant(
  mode: SshAuthMode,
): "success" | "secondary" {
  return mode === "node_key" ? "success" : "secondary";
}

export function getSshAuthModeChangeWarning(
  from: SshAuthMode,
  to: SshAuthMode,
): string | null {
  if (from === to || from !== "node_key") {
    return null;
  }

  const target = SSH_AUTH_MODE_LABELS[to];
  return `Switching to ${target} leaves node-local SSH keys in the node store until an operator runs nyxid node ssh-credentials prune --stale. Continue?`;
}
