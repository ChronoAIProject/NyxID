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
import {
  NYXID_CHAT_ADMISSION_UNKNOWN_MESSAGE,
  NyxIdChatSendError,
  nyxIdChatCommandErrorSchema,
  nyxIdChatEventSchema,
  nyxIdChatHistorySchema,
  nyxIdChatRecoverySchema,
  nyxIdChatRequestSchema,
  type NyxIdChatCompletedEvent,
  type NyxIdChatEvent,
  type NyxIdChatHistory,
  type NyxIdChatRecovery,
  type NyxIdChatRequest,
} from "./chat";
import { nyxIdViewSchema, parseNyxIdView, type NyxIdView } from "./nyxid";
import { windowDragEventSchema, type WindowDragEvent } from "./window-drag";

const STATE_CHANGED_EVENT = "companion://state-changed";
const MEAL_DUE_EVENT = "companion://meal-due";
const WINDOW_DRAG_EVENT = "companion://window-drag";
const NYXID_CHANGED_EVENT = "companion://nyxid-changed";
const NYXID_CHAT_EVENT = "companion://nyxid-chat";

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

  async startWindowDrag(): Promise<void> {
    await invoke("start_window_drag");
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

  async sendNyxIdChat(
    request: NyxIdChatRequest,
  ): Promise<NyxIdChatCompletedEvent> {
    const parsedRequest = nyxIdChatRequestSchema.parse(request);
    try {
      const result = nyxIdChatEventSchema.parse(
        await invoke<unknown>("send_nyxid_chat", { request: parsedRequest }),
      );
      if (result.kind !== "completed") {
        throw new Error("NyxID 对话返回了无效的结束状态");
      }
      return result;
    } catch (error) {
      if (error instanceof NyxIdChatSendError) throw error;
      const parsedError = nyxIdChatCommandErrorSchema.safeParse(error);
      if (parsedError.success) {
        throw new NyxIdChatSendError(
          parsedError.data.kind,
          parsedError.data.message,
        );
      }
      throw new NyxIdChatSendError(
        "admission_unknown",
        NYXID_CHAT_ADMISSION_UNKNOWN_MESSAGE,
      );
    }
  }

  async nyxIdChatHistory(conversationId: string): Promise<NyxIdChatHistory> {
    return nyxIdChatHistorySchema.parse(
      await invoke<unknown>("nyxid_chat_history", { conversationId }),
    );
  }

  async recoverNyxIdChat(): Promise<NyxIdChatRecovery | null> {
    const recovery = await invoke<unknown>("recover_nyxid_chat");
    return recovery === null ? null : nyxIdChatRecoverySchema.parse(recovery);
  }

  async stopNyxIdChat(conversationId: string): Promise<void> {
    await invoke("nyxid_chat_stop", { conversationId });
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

  onWindowDrag(listener: (event: WindowDragEvent) => void): Promise<Unlisten> {
    return listen<unknown>(WINDOW_DRAG_EVENT, (event) => {
      const parsed = windowDragEventSchema.safeParse(event.payload);
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

  onNyxIdChatEvent(
    listener: (event: NyxIdChatEvent) => void,
  ): Promise<Unlisten> {
    return listen<unknown>(NYXID_CHAT_EVENT, (event) => {
      const parsed = nyxIdChatEventSchema.safeParse(event.payload);
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
