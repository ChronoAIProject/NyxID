import type { CreateServicePoolInput, ServicePool } from "@/schemas/pools";

export function poolEditorDefaults(pool?: ServicePool): CreateServicePoolInput {
  return {
    slug: pool?.slug ?? "",
    name: pool?.name ?? "",
    description: pool?.description ?? "",
    strategy: pool?.strategy ?? "priority",
    tier_balance: pool?.tier_balance ?? "round_robin",
    member_contract: pool?.member_contract ?? "same_api",
    failover: pool?.failover ?? null,
    members: pool?.members ?? [],
    is_active: pool?.is_active ?? true,
  };
}

export function validPoolWeight(value: number | undefined): boolean {
  return (
    value !== undefined &&
    Number.isInteger(value) &&
    value >= 1 &&
    value <= 1000
  );
}

// Keep every routing mode's draft settings; submit only the active mode's fields.
export function poolEditorPayload(
  input: CreateServicePoolInput,
): CreateServicePoolInput {
  const priority = input.strategy === "priority";
  const weighted =
    input.strategy === "weighted" ||
    (priority && input.tier_balance === "weighted");
  const aiChat = priority && input.member_contract === "ai_chat";
  return {
    ...input,
    tier_balance: priority ? input.tier_balance : "round_robin",
    member_contract: aiChat ? "ai_chat" : "same_api",
    failover: priority ? input.failover : null,
    members: input.members.map((member) => ({
      ...member,
      weight: !weighted && !validPoolWeight(member.weight) ? 1 : member.weight,
      priority: priority ? member.priority : 0,
      model: aiChat ? member.model : null,
      same_api_compatible: priority ? member.same_api_compatible : false,
    })),
  };
}
