import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  activePromptSchema,
  choiceIdSchema,
  companionSettingsSchema,
  parseCompanionSnapshot,
  type CompanionSettings,
  type CompanionSnapshot,
  type MealId,
  type RecommendationMood,
} from "../domain/companion";
import type {
  CompanionRuntime,
  Unlisten,
  WindowMode,
} from "./companion-runtime";

const STATE_CHANGED_EVENT = "companion://state-changed";
const MEAL_DUE_EVENT = "companion://meal-due";

async function invokeSnapshot(
  command: string,
  args?: Record<string, unknown>,
): Promise<CompanionSnapshot> {
  return parseCompanionSnapshot(await invoke<unknown>(command, args));
}

export class TauriCompanionRuntime implements CompanionRuntime {
  snapshot(): Promise<CompanionSnapshot> {
    return invokeSnapshot("companion_snapshot");
  }

  saveSettings(settings: CompanionSettings): Promise<CompanionSnapshot> {
    return invokeSnapshot("save_companion_settings", {
      settings: companionSettingsSchema.parse(settings),
    });
  }

  snoozeMeal(mealId: MealId, minutes = 10): Promise<CompanionSnapshot> {
    return invokeSnapshot("snooze_meal", { mealId, minutes });
  }

  skipMeal(mealId: MealId): Promise<CompanionSnapshot> {
    return invokeSnapshot("skip_meal", { mealId });
  }

  completeMeal(
    mealId: MealId,
    choiceId?: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    return invokeSnapshot("complete_meal", {
      mealId,
      choiceId:
        choiceId === undefined ? undefined : choiceIdSchema.parse(choiceId),
      mood,
    });
  }

  dislikeSuggestion(
    mealId: MealId,
    choiceId: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    return invokeSnapshot("dislike_suggestion", {
      mealId,
      choiceId: choiceIdSchema.parse(choiceId),
      mood,
    });
  }

  setQuietMode(quietMode: boolean): Promise<CompanionSnapshot> {
    return invokeSnapshot("set_quiet_mode", { quietMode });
  }

  triggerDemoReminder(mealId?: MealId): Promise<CompanionSnapshot> {
    return invokeSnapshot("trigger_demo_reminder", { mealId });
  }

  async setWindowMode(mode: WindowMode): Promise<void> {
    await invoke("set_window_mode", { mode });
  }

  async openNyxidAssistant(): Promise<void> {
    await invoke("open_nyxid_assistant");
  }

  getLaunchAtLogin(): Promise<boolean> {
    return invoke<boolean>("get_launch_at_login");
  }

  setLaunchAtLogin(enabled: boolean): Promise<boolean> {
    return invoke<boolean>("set_launch_at_login", { enabled });
  }

  onStateChanged(
    listener: (snapshot: CompanionSnapshot) => void,
  ): Promise<Unlisten> {
    return listen<unknown>(STATE_CHANGED_EVENT, (event) => {
      listener(parseCompanionSnapshot(event.payload));
    });
  }

  onMealDue(
    listener: (prompt: { mealId: MealId; dueAt: string }) => void,
  ): Promise<Unlisten> {
    return listen<unknown>(MEAL_DUE_EVENT, (event) => {
      const parsed = activePromptSchema.safeParse(event.payload);
      if (parsed.success) {
        listener(parsed.data);
      }
    });
  }

  dispose(): void {}
}

export function isTauriHost(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
  );
}
