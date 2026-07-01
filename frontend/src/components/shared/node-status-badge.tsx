import { StatusBadge } from "@/components/shared/status-badge";

/**
 * Renders the canonical node status (Online / Draining / Offline) as a
 * tooltipped `<StatusBadge>` backed by the shared status registry.
 *
 * Node status is *derived* from two inputs — the live `isConnected`
 * heartbeat and the persisted `status` field — so we pre-compute the
 * registry key here. Callers keep the same `<NodeStatusBadge>` API; the
 * tooltip + (where applicable) remediation link come for free.
 *
 * Wave B item B.4.
 */
export function NodeStatusBadge({
  status,
  isConnected,
}: {
  readonly status: string;
  readonly isConnected: boolean;
}) {
  const statusKey = isConnected
    ? "Online"
    : status === "draining"
      ? "Draining"
      : "Offline";

  return <StatusBadge domain="node" statusKey={statusKey} />;
}
