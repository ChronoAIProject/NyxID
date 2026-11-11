import { translateUi } from "@/i18n/use-ui-translation";
import {
  defaultFailoverPolicy,
  type PoolCandidate,
  type ServicePool,
  type ServicePoolMember,
} from "@/schemas/pools";

export function poolStrategyLabel(pool: ServicePool): string {
  return pool.strategy === "priority"
    ? translateUi("Priority routing")
    : pool.strategy === "weighted"
      ? translateUi("Weighted rotation")
      : translateUi("Round-robin rotation");
}

export function poolFailoverLabel(pool: ServicePool): string {
  if (!pool.is_active) return translateUi("Pool disabled · no failover");
  if (pool.strategy !== "priority") return translateUi("Single attempt · no failover");
  const policy = pool.failover ?? defaultFailoverPolicy;
  return policy.max_attempts === 1 || !policy.retry_on.length
    ? translateUi("Failover off")
    : translateUi("Failover · up to {{count}} attempts", { count: policy.max_attempts });
}

function configuredAttempts(pool: ServicePool): number {
  if (!pool.is_active || !pool.members.some((member) => member.enabled))
    return 0;
  if (pool.strategy !== "priority") return 1;
  const policy = pool.failover ?? defaultFailoverPolicy;
  return policy.retry_on.length ? policy.max_attempts : 1;
}

export function poolFailoverSummary(pools: readonly ServicePool[]): string {
  if (!pools.length) return translateUi("No pool");
  if (pools.length === 1) {
    const pool = pools[0]!;
    if (!pool.is_active) return translateUi("Pool disabled");
    const attempts = configuredAttempts(pool);
    if (!attempts) return translateUi("No enabled members");
    return attempts > 1 ? translateUi("Up to {{count}} attempts", { count: attempts }) : translateUi("Off · single attempt");
  }
  if (pools.every((pool) => !pool.is_active)) return translateUi("Pools disabled");
  const enabled = pools.filter((pool) => configuredAttempts(pool) > 1).length;
  return enabled
    ? translateUi("On in {{enabled}} of {{count}} pools", { enabled, count: pools.length })
    : translateUi("Off in all pools");
}

export function orderedPoolMembers(
  pool: Pick<ServicePool, "strategy" | "members">,
): ServicePoolMember[] {
  return pool.strategy === "priority"
    ? [...pool.members].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0))
    : [...pool.members];
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
  if (!member.enabled) return translateUi("Member disabled");
  if (!candidate) return translateUi("Not inspected");
  if (candidate.reason)
    return translateUi(reasons[candidate.reason] ?? candidate.reason.replaceAll("_", " "));
  return candidate.eligible ? translateUi("Eligible") : translateUi("Not eligible");
}
