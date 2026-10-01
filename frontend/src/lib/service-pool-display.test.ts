import { describe, expect, it } from "vitest";
import { defaultFailoverPolicy, type ServicePool } from "@/schemas/pools";
import {
  orderedPoolMembers,
  poolFailoverLabel,
  poolFailoverSummary,
  poolMemberStatus,
  reorderPoolMembers,
} from "./service-pool-display";
const members = [
  {
    user_service_id: "backup",
    enabled: true,
    priority: 20,
    weight: 4,
    model: "model-b",
    same_api_compatible: true,
  },
  {
    user_service_id: "primary",
    enabled: true,
    priority: 0,
    weight: 2,
    model: "model-a",
  },
  {
    user_service_id: "disabled",
    enabled: false,
    priority: 10,
    weight: 1,
    model: null,
  },
];
const pool = {
  strategy: "priority",
  members,
  failover: null,
  is_active: true,
} as ServicePool;
describe("saved pool presentation", () => {
  it("summarizes configured failover without treating rotation or disabled pools as backups", () => {
    expect(poolFailoverSummary([pool])).toBe("Up to 3 attempts");
    const rotation = { ...pool, strategy: "weighted" as const };
    const disabled = { ...pool, is_active: false };
    expect(poolFailoverSummary([rotation])).toBe("Off · single attempt");
    expect(poolFailoverSummary([disabled])).toBe("Pool disabled");
    expect(poolFailoverLabel(disabled)).toBe("Pool disabled · no failover");
    expect(poolFailoverSummary([pool, rotation, disabled])).toBe(
      "On in 1 of 3 pools",
    );
    expect(poolFailoverSummary([rotation, disabled])).toBe("Off in all pools");
    expect(poolFailoverSummary([disabled, disabled])).toBe("Pools disabled");
    expect(
      poolFailoverSummary([
        {
          ...pool,
          members: members.map((member) => ({ ...member, enabled: false })),
        },
      ]),
    ).toBe("No enabled members");
    for (const failover of [
      { ...defaultFailoverPolicy, max_attempts: 1 },
      { ...defaultFailoverPolicy, retry_on: [] },
    ])
      expect(poolFailoverSummary([{ ...pool, failover }])).toBe(
        "Off · single attempt",
      );
  });
  it("uses default failover for null policy and honors disabled retry policies", () => {
    expect(poolFailoverLabel(pool)).toBe("Failover · up to 3 attempts");
    expect(
      poolFailoverLabel({
        ...pool,
        failover: { ...defaultFailoverPolicy, max_attempts: 1 },
      }),
    ).toBe("Failover off");
    expect(
      poolFailoverLabel({
        ...pool,
        failover: { ...defaultFailoverPolicy, retry_on: [] },
      }),
    ).toBe("Failover off");
    for (const strategy of ["round_robin", "weighted"] as const)
      expect(poolFailoverLabel({ ...pool, strategy })).toBe(
        "Single attempt · no failover",
      );
  });
  it("orders priority tiers numerically, preserves ties and leaves rotation order unchanged", () => {
    expect(orderedPoolMembers(pool).map((m) => m.user_service_id)).toEqual([
      "primary",
      "disabled",
      "backup",
    ]);
    expect(orderedPoolMembers({ ...pool, strategy: "round_robin" })).toEqual(
      members,
    );
    expect(
      orderedPoolMembers({
        strategy: "priority",
        members: members.map((member) => ({ ...member, priority: 0 })),
      }).map((m) => m.user_service_id),
    ).toEqual(["backup", "primary", "disabled"]);
  });
  it("reorders only real members and preserves model, weight, exclusion and compatibility fields", () => {
    const result = reorderPoolMembers(pool, "backup", "primary");
    expect(result).toEqual([
      { ...members[0], priority: 0 },
      { ...members[1], priority: 1 },
      { ...members[2], priority: 2 },
    ]);
    expect(members[0]!.priority).toBe(20);
    expect(reorderPoolMembers(pool, "foreign", "primary")).toEqual(
      orderedPoolMembers(pool),
    );
    expect(
      reorderPoolMembers(
        { ...pool, strategy: "weighted" },
        "backup",
        "primary",
      ),
    ).toEqual(members);
  });
  it("does not infer health from an enabled member", () => {
    expect(poolMemberStatus(members[0]!)).toBe("Not inspected");
    expect(poolMemberStatus(members[2]!)).toBe("Member disabled");
  });
});
