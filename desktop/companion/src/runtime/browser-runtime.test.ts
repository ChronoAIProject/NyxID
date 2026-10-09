import { describe, expect, it, vi } from "vitest";

import { createDefaultCompanionSettings } from "../domain/companion";
import {
  BROWSER_SNAPSHOT_KEY,
  BrowserCompanionRuntime,
  NYXID_ASSISTANT_URL,
  type RuntimeStorage,
} from "./browser-runtime";

class MemoryStorage implements RuntimeStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

const utcTimezone = () => "UTC";

describe("browser companion runtime", () => {
  it("persists settings and actions across runtime instances", async () => {
    const storage = new MemoryStorage();
    let currentTime = new Date("2026-10-09T12:31:00.000Z");
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const now = () => currentTime;
    const first = new BrowserCompanionRuntime({
      storage,
      now,
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    const settings = {
      ...createDefaultCompanionSettings("UTC"),
      companionName: "Orbit",
      onboardingComplete: true,
    };

    await first.saveSettings(settings);
    tick?.();
    const snoozed = await first.snoozeMeal("lunch", 20);
    expect(snoozed.runtime.activePrompt).toBeUndefined();
    currentTime = new Date("2026-10-09T12:51:00.000Z");
    tick?.();
    await first.completeMeal("lunch", "lemon-lentil-soup", "light");
    await first.setLaunchAtLogin(true);
    first.dispose();

    const second = new BrowserCompanionRuntime({
      storage,
      now,
      timezone: utcTimezone,
      autoStart: false,
    });
    const snapshot = await second.snapshot();
    expect(snapshot.settings.companionName).toBe("Orbit");
    expect(snapshot.history.at(-1)).toMatchObject({
      mealId: "lunch",
      action: "accepted",
      choiceId: "lemon-lentil-soup",
      mood: "light",
    });
    expect(snapshot.runtime.activePrompt).toBeUndefined();
    expect(await second.getLaunchAtLogin()).toBe(true);
    expect(storage.getItem(BROWSER_SNAPSHOT_KEY)).not.toBeNull();
  });

  it("emits state and meal events when the browser timer finds a due meal", async () => {
    const storage = new MemoryStorage();
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage,
      now: () => new Date("2026-10-09T12:31:00.000Z"),
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("UTC"),
      onboardingComplete: true,
    });
    const stateListener = vi.fn();
    const mealListener = vi.fn();
    await runtime.onStateChanged(stateListener);
    await runtime.onMealDue(mealListener);

    tick?.();

    expect(stateListener).toHaveBeenCalledTimes(1);
    expect(mealListener).toHaveBeenCalledWith({
      mealId: "lunch",
      dueAt: "2026-10-09T12:30:00.000Z",
    });
    expect((await runtime.snapshot()).runtime.activePrompt?.mealId).toBe(
      "lunch",
    );
    runtime.dispose();
  });

  it("recovers from corrupt persisted JSON", async () => {
    const storage = new MemoryStorage();
    storage.setItem(BROWSER_SNAPSHOT_KEY, "{broken");
    const runtime = new BrowserCompanionRuntime({
      storage,
      timezone: utcTimezone,
      autoStart: false,
    });

    expect((await runtime.snapshot()).schemaVersion).toBe(1);
    expect((await runtime.snapshot()).history).toEqual([]);
  });

  it("refreshes a restored snapshot to the current timezone", async () => {
    const storage = new MemoryStorage();
    storage.setItem(
      BROWSER_SNAPSHOT_KEY,
      JSON.stringify({
        schemaVersion: 1,
        settings: {
          ...createDefaultCompanionSettings("UTC"),
          onboardingComplete: true,
        },
        history: [],
        runtime: {
          activePrompt: {
            mealId: "breakfast",
            dueAt: "2026-10-09T08:00:00.000Z",
          },
          snoozedUntil: "2026-10-09T08:20:00.000Z",
          lastPromptKey: "2026-10-09:breakfast",
        },
      }),
    );

    const runtime = new BrowserCompanionRuntime({
      storage,
      timezone: () => "Asia/Shanghai",
      autoStart: false,
    });

    const restored = await runtime.snapshot();
    expect(restored.settings.timezone).toBe("Asia/Shanghai");
    expect(restored.runtime).toEqual({});
    expect(
      JSON.parse(storage.getItem(BROWSER_SNAPSHOT_KEY) ?? "null"),
    ).toMatchObject({
      settings: { timezone: "Asia/Shanghai" },
      runtime: {},
    });
  });

  it("refreshes a long-running timezone before evaluating the current meal", async () => {
    let currentTime = new Date("2026-10-09T00:01:00.000Z");
    let currentTimezone = "Asia/Shanghai";
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => currentTime,
      timezone: () => currentTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("Asia/Shanghai"),
      onboardingComplete: true,
    });
    tick?.();
    await runtime.snoozeMeal("breakfast", 120);
    const stateListener = vi.fn();
    const mealListener = vi.fn();
    await runtime.onStateChanged(stateListener);
    await runtime.onMealDue(mealListener);

    currentTime = new Date("2026-10-09T00:31:00.000Z");
    currentTimezone = "Etc/GMT-12";
    tick?.();

    const snapshot = await runtime.snapshot();
    expect(snapshot.settings.timezone).toBe("Etc/GMT-12");
    expect(snapshot.runtime.snoozedUntil).toBeUndefined();
    expect(snapshot.runtime.activePrompt).toEqual({
      mealId: "lunch",
      dueAt: "2026-10-09T00:30:00.000Z",
    });
    expect(snapshot.runtime.lastPromptKey).toBe("2026-10-09:lunch");
    expect(stateListener).toHaveBeenCalledTimes(1);
    expect(mealListener).toHaveBeenCalledWith(snapshot.runtime.activePrompt);
    runtime.dispose();
  });

  it("keeps demo actions out of adaptation history without consuming a real meal", async () => {
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => new Date("2026-10-09T10:00:00.000Z"),
      timezone: utcTimezone,
      autoStart: false,
    });

    await runtime.triggerDemoReminder("dinner");
    const settled = await runtime.completeMeal(
      "dinner",
      "lemon-lentil-soup",
      "balanced",
    );

    expect(settled.history).toEqual([]);
    expect(settled.runtime.activePrompt).toBeUndefined();
    expect(settled.runtime.lastPromptKey).toMatch(/^demo:dinner:/);
  });

  it("keeps demo dislikes out of history while leaving the prompt active", async () => {
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => new Date("2026-10-09T10:00:00.000Z"),
      timezone: utcTimezone,
      autoStart: false,
    });
    await runtime.triggerDemoReminder("lunch");

    const snapshot = await runtime.dislikeSuggestion(
      "lunch",
      "sesame-tofu-rice",
      "balanced",
    );

    expect(snapshot.history).toEqual([]);
    expect(snapshot.runtime.activePrompt?.mealId).toBe("lunch");
  });

  it("records real disliked suggestions without settling the meal", async () => {
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => new Date("2026-10-09T12:31:00.000Z"),
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("UTC"),
      onboardingComplete: true,
    });
    tick?.();

    const snapshot = await runtime.dislikeSuggestion(
      "lunch",
      "sesame-tofu-rice",
      "balanced",
    );

    expect(snapshot.history.at(-1)).toMatchObject({
      mealId: "lunch",
      action: "disliked",
      choiceId: "sesame-tofu-rice",
      mood: "balanced",
    });
    expect(snapshot.runtime.activePrompt?.mealId).toBe("lunch");
    runtime.dispose();
  });

  it("clears a snooze when that meal is disabled", async () => {
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => new Date("2026-10-09T12:31:00.000Z"),
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("UTC"),
      onboardingComplete: true,
    });
    tick?.();
    await runtime.snoozeMeal("lunch", 10);

    const before = await runtime.snapshot();
    const saved = await runtime.saveSettings({
      ...before.settings,
      meals: before.settings.meals.map((meal) =>
        meal.id === "lunch" ? { ...meal, enabled: false } : meal,
      ),
    });

    expect(saved.runtime.snoozedUntil).toBeUndefined();
    expect(saved.runtime.activePrompt).toBeUndefined();
    runtime.dispose();
  });

  it("expires a stale prompt so a later meal can become due", async () => {
    let currentTime = new Date("2026-10-09T08:01:00.000Z");
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => currentTime,
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("UTC"),
      onboardingComplete: true,
    });
    tick?.();
    expect((await runtime.snapshot()).runtime.activePrompt?.mealId).toBe(
      "breakfast",
    );

    currentTime = new Date("2026-10-09T12:31:00.000Z");
    tick?.();

    expect((await runtime.snapshot()).runtime.activePrompt?.mealId).toBe(
      "lunch",
    );
    runtime.dispose();
  });

  it("expires a stale breakfast snooze without blocking current lunch", async () => {
    let currentTime = new Date("2026-10-09T08:01:00.000Z");
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => currentTime,
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    await runtime.saveSettings({
      ...createDefaultCompanionSettings("UTC"),
      onboardingComplete: true,
    });
    tick?.();
    await runtime.snoozeMeal("breakfast", 10);

    currentTime = new Date("2026-10-09T12:31:00.000Z");
    tick?.();

    const snapshot = await runtime.snapshot();
    expect(snapshot.runtime.activePrompt?.mealId).toBe("lunch");
    expect(snapshot.runtime.snoozedUntil).toBeUndefined();
    expect(snapshot.runtime.lastPromptKey).toBe("2026-10-09:lunch");
    runtime.dispose();
  });

  it("keeps the original meal date when a snooze crosses midnight", async () => {
    let currentTime = new Date("2026-10-09T23:56:00.000Z");
    let tick: (() => void) | undefined;
    const timerHandle = globalThis.setInterval(() => undefined, 60_000);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      now: () => currentTime,
      timezone: utcTimezone,
      setInterval: (callback) => {
        tick = callback;
        return timerHandle;
      },
      clearInterval: (handle) => globalThis.clearInterval(handle),
    });
    const defaults = createDefaultCompanionSettings("UTC");
    await runtime.saveSettings({
      ...defaults,
      onboardingComplete: true,
      meals: defaults.meals.map((meal) => ({
        ...meal,
        enabled: meal.id === "dinner",
        time: meal.id === "dinner" ? "23:55" : meal.time,
      })),
    });
    tick?.();
    await runtime.snoozeMeal("dinner", 10);
    currentTime = new Date("2026-10-10T00:06:00.000Z");
    tick?.();

    const settled = await runtime.completeMeal(
      "dinner",
      "lemon-lentil-soup",
      "light",
    );

    expect(settled.history.at(-1)?.dateKey).toBe("2026-10-09");
    expect(settled.runtime.lastPromptKey).toBe("2026-10-09:dinner");
    runtime.dispose();
  });

  it("matches native behavior when no meal is enabled for a demo", async () => {
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      timezone: utcTimezone,
      autoStart: false,
    });
    const settings = createDefaultCompanionSettings("UTC");
    await runtime.saveSettings({
      ...settings,
      meals: settings.meals.map((meal) => ({ ...meal, enabled: false })),
    });

    await expect(runtime.triggerDemoReminder()).rejects.toThrow(
      "No configured meal",
    );
  });

  it("opens the NyxID assistant with opener isolation", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const runtime = new BrowserCompanionRuntime({
      storage: new MemoryStorage(),
      timezone: utcTimezone,
      autoStart: false,
    });

    await runtime.openNyxidAssistant();

    expect(open).toHaveBeenCalledWith(
      NYXID_ASSISTANT_URL,
      "_blank",
      "noopener,noreferrer",
    );
  });
});
