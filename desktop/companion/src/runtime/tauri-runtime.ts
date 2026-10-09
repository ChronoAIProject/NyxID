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
import { nyxIdViewSchema, parseNyxIdView, type NyxIdView } from "./nyxid";

const STATE_CHANGED_EVENT = "companion://state-changed";
const MEAL_DUE_EVENT = "companion://meal-due";
const NYXID_CHANGED_EVENT = "companion://nyxid-changed";

async function invokeSnapshot(
  command: string,
  args?: Record<string, unknown>,
): Promise<CompanionSnapshot> {
  return parseCompanionSnapshot(await invoke<unknown>(command, args));
}

async function invokeNyxId(command: string): Promise<NyxIdView> {
  return parseNyxIdView(await invoke<unknown>(command));
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

  nyxidStatus(): Promise<NyxIdView> {
    return invokeNyxId("nyxid_status");
  }

  startNyxidLogin(): Promise<NyxIdView> {
    return invokeNyxId("start_nyxid_login");
  }

  cancelNyxidLogin(): Promise<NyxIdView> {
    return invokeNyxId("cancel_nyxid_login");
  }

  refreshNyxidCapabilities(): Promise<NyxIdView> {
    return invokeNyxId("refresh_nyxid_capabilities");
  }

  logoutNyxid(): Promise<NyxIdView> {
    return invokeNyxId("logout_nyxid");
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

  onNyxidChanged(listener: (view: NyxIdView) => void): Promise<Unlisten> {
    return listen<unknown>(NYXID_CHANGED_EVENT, (event) => {
      const parsed = nyxIdViewSchema.safeParse(event.payload);
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
