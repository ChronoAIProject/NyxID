import {
  defaultFailoverPolicy,
  type PoolCandidate,
  type ServicePool,
  type ServicePoolMember,
} from "@/schemas/pools";

export function poolStrategyLabel(pool: ServicePool): string {
  return pool.strategy === "priority"
    ? "Priority routing"
    : pool.strategy === "weighted"
      ? "Weighted rotation"
      : "Round-robin rotation";
}

export function poolFailoverLabel(pool: ServicePool): string {
  if (pool.strategy !== "priority") return "Single attempt · no failover";
  const policy = pool.failover ?? defaultFailoverPolicy;
  return policy.max_attempts === 1 || !policy.retry_on.length
    ? "Failover off"
    : `Failover · up to ${policy.max_attempts} attempts`;
}

export function orderedPoolMembers(
  pool: Pick<ServicePool, "strategy" | "members">,
): ServicePoolMember[] {
  return pool.strategy === "priority"
    ? [...pool.members].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0))
    : [...pool.members];
}

/** Reordering explicitly creates a strict priority sequence; equal tiers are edited numerically. */
export function reorderPoolMembers(
  pool: Pick<ServicePool, "strategy" | "members">,
  from: string,
  to: string,
): ServicePoolMember[] {
  const members = orderedPoolMembers(pool);
  const start = members.findIndex((m) => m.user_service_id === from);
  const end = members.findIndex((m) => m.user_service_id === to);
  if (pool.strategy !== "priority" || start < 0 || end < 0 || start === end)
    return members;
  members.splice(end, 0, members.splice(start, 1)[0]!);
  return members.map((member, index) => ({ ...member, priority: index }));
}

const reasons: Record<string, string> = {
  unavailable: "Connection unavailable",
  inactive: "Service disabled",
  disabled: "Member disabled",
  cooldown: "Cooling down",
  incompatible_protocol: "Incompatible protocol",
  compatibility_declaration_required: "Compatibility confirmation needed",
  inference_protocol_required: "Inference metadata required",
  operation_unsupported: "Operation not permitted",
  node_upgrade_required: "Node upgrade required",
  node_offline: "Node offline",
  unsupported_transport: "Unsupported transport",
};
export function poolMemberStatus(
  member: ServicePoolMember,
  candidate?: PoolCandidate,
): string {
  if (!member.enabled) return "Member disabled";
  if (!candidate) return "Not inspected";
  if (candidate.reason)
    return reasons[candidate.reason] ?? candidate.reason.replaceAll("_", " ");
  return candidate.eligible ? "Eligible" : "Not eligible";
}
