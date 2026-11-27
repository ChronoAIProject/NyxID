import { describe, it, expect } from "vitest";
import fixture from "../../../../shared/agent-avatars/mood-fixture.json";
import {
  avatarMood,
  endCelebration,
  settleAvatarMood,
  settledAvatarMood,
  type AvatarSettledReply,
  type AvatarSettlement,
  type AvatarMoodInput,
} from "./agent-avatar-model";
describe("avatar mood", () => {
  it.each(fixture.cases)("$name", ({ input, mood }) => {
    expect(avatarMood(input as AvatarMoodInput)).toBe(mood);
  });

  it.each(fixture.sequences)("settlement: $name", ({ steps }) => {
    let settlement: AvatarSettlement | undefined;
    for (const step of steps as readonly {
      conversation?: string;
      active: boolean;
      reply: AvatarSettledReply | null;
      mood: string;
      expire?: boolean;
    }[]) {
      const observed = {
        conversationId: step.conversation ?? "c",
        active: step.active,
        reply: step.reply,
      };
      settlement = settleAvatarMood(settlement, observed);
      if (step.expire) settlement = endCelebration(settlement);
      expect(
        settledAvatarMood(
          { activeTurn: step.active ? {} : null },
          settlement,
          step.reply,
        ),
      ).toBe(step.mood);
    }
  });

  it("returns the same settlement object when nothing changed", () => {
    const observed = {
      conversationId: "c",
      active: false,
      reply: { id: "r", outcome: "completed" as const },
    };
    const first = settleAvatarMood(undefined, observed);
    expect(settleAvatarMood(first, observed)).toBe(first);
  });
});
