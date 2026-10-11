import { describe, expect, it } from "vitest";

import { createDefaultCompanionSettings, type HistoryEntry } from "./companion";
import { recommendMeals } from "./recommendations";

describe("local recommendation scoring", () => {
  it("is deterministic for the same local inputs", () => {
    const input = {
      mealId: "dinner" as const,
      mood: "balanced" as const,
      settings: {
        ...createDefaultCompanionSettings("UTC"),
        budget: "flexible" as const,
      },
      history: [],
      dateKey: "2026-10-09",
    };

    expect(recommendMeals(input)).toEqual(recommendMeals(input));
  });

  it("filters choices using budget, hard dietary restrictions, and avoids", () => {
    const result = recommendMeals({
      mealId: "dinner",
      mood: "balanced",
      settings: {
        ...createDefaultCompanionSettings("UTC"),
        budget: "everyday",
        dietary: ["vegan"],
        avoid: ["sesame allergy"],
      },
      history: [],
      dateKey: "2026-10-09",
      limit: 6,
    });
    const selected = [result.primary, ...result.alternatives].filter(
      (choice) => choice !== undefined,
    );

    expect(selected.length).toBeGreaterThan(0);
    expect(selected.every((choice) => choice.cost !== "flexible")).toBe(true);
    expect(
      selected.every((choice) => choice.suitableFor.includes("vegan")),
    ).toBe(true);
    expect(selected.every((choice) => !choice.signals.includes("sesame"))).toBe(
      true,
    );
    expect(result.excluded).toContainEqual({
      id: "sesame-tofu-rice",
      reason: "avoid",
    });
  });

  it("moves a disliked choice down while exposing the feedback reason", () => {
    const settings = {
      ...createDefaultCompanionSettings("UTC"),
      budget: "flexible" as const,
    };
    const first = recommendMeals({
      mealId: "breakfast",
      mood: "balanced",
      settings,
      history: [],
      dateKey: "2026-10-09",
    }).primary;
    expect(first).toBeDefined();

    const feedback: HistoryEntry = {
      mealId: "breakfast",
      dateKey: "2026-10-09",
      action: "disliked",
      choiceId: first!.id,
      mood: "balanced",
      at: "2026-10-09T12:35:00.000Z",
    };
    const adapted = recommendMeals({
      mealId: "breakfast",
      mood: "balanced",
      settings,
      history: [feedback],
      dateKey: "2026-10-10",
      limit: 6,
    });
    const priorChoice = [adapted.primary, ...adapted.alternatives].find(
      (choice) => choice?.id === first!.id,
    );

    expect(adapted.primary?.id).not.toBe(first!.id);
    expect(priorChoice?.reasons).toContainEqual({
      code: "disliked_before",
      points: -72,
    });
  });

  it("scores the product soft-preference controls and understands Chinese avoids", () => {
    const result = recommendMeals({
      mealId: "dinner",
      mood: "balanced",
      settings: {
        ...createDefaultCompanionSettings("UTC"),
        budget: "flexible",
        dietary: ["light", "high-protein", "low-oil", "mild"],
        avoid: ["不要芝麻"],
      },
      history: [],
      dateKey: "2026-10-09",
      limit: 6,
    });

    expect(result.primary?.id).toBe("salmon-greens-plate");
    expect(result.primary?.reasons).toContainEqual({
      code: "preference_match",
      points: 36,
    });
    expect(result.excluded).toContainEqual({
      id: "sesame-tofu-rice",
      reason: "avoid",
    });
  });
});
