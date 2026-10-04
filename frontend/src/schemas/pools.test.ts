import { describe, expect, it } from "vitest";
import { createServicePoolSchema, updateServicePoolSchema } from "./pools";

const input = {
  name: "Pool",
  slug: "pool",
  strategy: "priority" as const,
  member_contract: "ai_chat" as const,
  members: [
    { user_service_id: "member", weight: 1, enabled: true, model: "model" },
  ],
};

describe("pool Unicode limits", () => {
  it.each(["a", "服", "🪐"])(
    "matches server character boundaries for %s",
    (character) => {
      const boundary = {
        ...input,
        name: character.repeat(128),
        description: character.repeat(1024),
        members: [{ ...input.members[0]!, model: character.repeat(256) }],
      };
      for (const schema of [createServicePoolSchema, updateServicePoolSchema]) {
        expect(schema.safeParse(boundary).success).toBe(true);
        expect(
          schema.safeParse({ ...boundary, name: character.repeat(129) })
            .success,
        ).toBe(false);
        expect(
          schema.safeParse({ ...boundary, description: character.repeat(1025) })
            .success,
        ).toBe(false);
        expect(
          schema.safeParse({
            ...boundary,
            members: [{ ...input.members[0]!, model: character.repeat(257) }],
          }).success,
        ).toBe(false);
      }
    },
  );
});
