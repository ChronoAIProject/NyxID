import { describe, expect, it } from "vitest";

import {
  COMPANION_SCHEMA_VERSION,
  companionSnapshotSchema,
  parseCompanionSnapshot,
} from "./companion";

describe("companion snapshot recovery", () => {
  it("migrates prototype fields and repairs invalid fields independently", () => {
    const snapshot = parseCompanionSnapshot(
      {
        schemaVersion: 0,
        preferences: {
          companionName: "  Orbit  ",
          userName: " Sam ",
          timezone: "Not/AZone",
          budget: "low",
          dietary: [" vegan ", "Vegan", 42],
          avoid: [" peanuts ", ""],
          quietMode: true,
          onboardingComplete: true,
          meals: {
            breakfast: { label: "Morning", time: "7:3", enabled: false },
            lunch: { label: "Midday", time: "13:05", enabled: true },
          },
        },
        history: [
          {
            meal: "lunch",
            outcome: "completed",
            occurredAt: "2026-10-09T05:05:00.000Z",
            choiceId: " sesame-tofu-rice ",
            mood: "balanced",
          },
          { mealId: "snack", action: "accepted", at: "not-a-date" },
        ],
        runtime: {
          activePrompt: null,
          snoozedUntil: "not-a-date",
          lastPromptKey: "  2026-10-09:lunch  ",
        },
      },
      "UTC",
    );

    expect(snapshot.schemaVersion).toBe(COMPANION_SCHEMA_VERSION);
    expect(snapshot.settings).toMatchObject({
      companionName: "Orbit",
      userName: "Sam",
      timezone: "UTC",
      budget: "low",
      dietary: ["vegan"],
      avoid: ["peanuts"],
      quietMode: true,
      onboardingComplete: true,
    });
    expect(snapshot.settings.meals).toEqual([
      { id: "breakfast", label: "Morning", time: "08:00", enabled: false },
      { id: "lunch", label: "Midday", time: "13:05", enabled: true },
      { id: "dinner", label: "Dinner", time: "18:30", enabled: true },
    ]);
    expect(snapshot.history).toEqual([
      {
        mealId: "lunch",
        dateKey: "2026-10-09",
        action: "accepted",
        choiceId: "sesame-tofu-rice",
        mood: "balanced",
        at: "2026-10-09T05:05:00.000Z",
      },
    ]);
    expect(snapshot.runtime).toEqual({ lastPromptKey: "2026-10-09:lunch" });
  });

  it("retains only the newest 120 valid history rows", () => {
    const history = Array.from({ length: 125 }, (_, index) => ({
      mealId: "breakfast",
      dateKey: "2026-10-09",
      action: "accepted",
      choiceId: `choice-${index}`,
      at: `2026-10-09T00:${String(index % 60).padStart(2, "0")}:00.000Z`,
    }));
    const snapshot = parseCompanionSnapshot({ history }, "UTC");

    expect(snapshot.history).toHaveLength(120);
    expect(snapshot.history[0]?.choiceId).toBe("choice-5");
    expect(companionSnapshotSchema.safeParse(snapshot).success).toBe(true);
  });
});
