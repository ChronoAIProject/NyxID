import {
  choiceIdSchema,
  companionSettingsSchema,
  createDefaultCompanionSnapshot,
  isValidTimezone,
  mealIdSchema,
  parseCompanionSnapshot,
  type ActivePrompt,
  type CompanionSettings,
  type CompanionSnapshot,
  type HistoryEntry,
  type MealId,
  type RecommendationMood,
} from "../domain/companion";
import { dateKeyAt, findDueMeal, promptKey } from "../domain/meal-clock";
import type {
  CompanionRuntime,
  Unlisten,
  WindowMode,
} from "./companion-runtime";
import {
  NyxIdChatSendError,
  type NyxIdChatCompletedEvent,
  type NyxIdChatEvent,
  type NyxIdChatHistory,
  type NyxIdChatRecovery,
  type NyxIdChatRequest,
} from "./chat";
import type { NyxIdView } from "./nyxid";
import type { WindowDragEvent } from "./window-drag";

export const BROWSER_SNAPSHOT_KEY = "nyxid.companion.snapshot.v1";
export const BROWSER_LAUNCH_AT_LOGIN_KEY = "nyxid.companion.launch-at-login.v1";
export const NYXID_ASSISTANT_URL = "https://nyx.chrono-ai.fun/assistant";
const DUE_WINDOW_MS = 45 * 60_000;

export interface RuntimeStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

type TimerHandle = ReturnType<typeof globalThis.setInterval> | number;

export interface BrowserRuntimeOptions {
  storage?: RuntimeStorage;
  now?: () => Date;
  timezone?: () => string;
  setInterval?: (callback: () => void, milliseconds: number) => TimerHandle;
  clearInterval?: (handle: TimerHandle) => void;
  timerIntervalMs?: number;
  autoStart?: boolean;
}

function availableLocalStorage(): RuntimeStorage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

function browserTimezone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone ?? "";
}

function detectedTimezone(provider: () => string): string | undefined {
  try {
    const timezone = provider();
    return typeof timezone === "string" && isValidTimezone(timezone)
      ? timezone
      : undefined;
  } catch {
    return undefined;
  }
}

function refreshSnapshotTimezone(
  snapshot: CompanionSnapshot,
  timezone: string | undefined,
): CompanionSnapshot {
  if (!timezone || timezone === snapshot.settings.timezone) {
    return snapshot;
  }

  return {
    ...snapshot,
    settings: { ...snapshot.settings, timezone },
    runtime: {},
  };
}

function cloneSnapshot(snapshot: CompanionSnapshot): CompanionSnapshot {
  return parseCompanionSnapshot(snapshot, snapshot.settings.timezone);
}

function validatedSnoozeMinutes(value: number): number {
  if (!Number.isInteger(value) || value < 1 || value > 120) {
    throw new Error("Snooze minutes must be between 1 and 120");
  }
  return value;
}

function mealIdFromPromptKey(key: string | undefined): MealId | undefined {
  if (!key) {
    return undefined;
  }
  const candidate = key.startsWith("demo:")
    ? key.split(":")[1]
    : key.split(":").at(-1);
  const parsed = mealIdSchema.safeParse(candidate);
  return parsed.success ? parsed.data : undefined;
}

function dateKeyFromPromptKey(
  key: string | undefined,
  mealId: MealId,
): string | undefined {
  if (!key || key.startsWith("demo:")) {
    return undefined;
  }
  const match = /^(\d{4}-\d{2}-\d{2}):(breakfast|lunch|dinner)$/.exec(key);
  if (!match || match[2] !== mealId || !match[1]) {
    return undefined;
  }
  const [year, month, day] = match[1].split("-").map(Number);
  if (year === undefined || month === undefined || day === undefined) {
    return undefined;
  }
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day
    ? match[1]
    : undefined;
}

export class BrowserCompanionRuntime implements CompanionRuntime {
  private state: CompanionSnapshot;
  private readonly storage: RuntimeStorage | undefined;
  private readonly now: () => Date;
  private readonly timezone: () => string;
  private readonly clearTimer: (handle: TimerHandle) => void;
  private readonly stateListeners = new Set<
    (snapshot: CompanionSnapshot) => void
  >();
  private readonly mealListeners = new Set<(prompt: ActivePrompt) => void>();
  private timer: TimerHandle | undefined;

  constructor(options: BrowserRuntimeOptions = {}) {
    this.storage = options.storage ?? availableLocalStorage();
    this.now = options.now ?? (() => new Date());
    this.timezone = options.timezone ?? browserTimezone;
    this.clearTimer = options.clearInterval ?? globalThis.clearInterval;
    const timezone = detectedTimezone(this.timezone);
    const restored = this.readStoredSnapshot(timezone);
    this.state = refreshSnapshotTimezone(restored, timezone);
    if (this.state !== restored) {
      this.persist();
    }

    if (options.autoStart !== false) {
      const schedule = options.setInterval ?? globalThis.setInterval;
      this.timer = schedule(
        () => this.checkSchedule(),
        Math.max(1_000, options.timerIntervalMs ?? 30_000),
      );
    }
  }

  private readStoredSnapshot(timezone: string | undefined): CompanionSnapshot {
    const fallbackTimezone = timezone ?? "UTC";
    try {
      const stored = this.storage?.getItem(BROWSER_SNAPSHOT_KEY);
      return stored
        ? parseCompanionSnapshot(JSON.parse(stored), fallbackTimezone)
        : createDefaultCompanionSnapshot(fallbackTimezone);
    } catch {
      return createDefaultCompanionSnapshot(fallbackTimezone);
    }
  }

  private persist(): void {
    try {
      this.storage?.setItem(BROWSER_SNAPSHOT_KEY, JSON.stringify(this.state));
    } catch {
      // Browser privacy modes can reject localStorage writes. In-memory state
      // remains functional for the current development session.
    }
  }

  private commit(next: CompanionSnapshot): CompanionSnapshot {
    this.state = parseCompanionSnapshot(next, next.settings.timezone);
    this.persist();
    const snapshot = cloneSnapshot(this.state);
    for (const listener of this.stateListeners) {
      listener(cloneSnapshot(snapshot));
    }
    return snapshot;
  }

  private emitMealDue(prompt: ActivePrompt): void {
    for (const listener of this.mealListeners) {
      listener({ ...prompt });
    }
  }

  private checkSchedule(): void {
    const now = this.now();
    let candidateState = refreshSnapshotTimezone(
      this.state,
      detectedTimezone(this.timezone),
    );
    const timezoneChanged = candidateState !== this.state;
    const { settings } = candidateState;
    let { runtime } = candidateState;
    let runtimeChanged = false;
    if (settings.quietMode || !settings.onboardingComplete) {
      if (timezoneChanged) {
        this.commit(candidateState);
      }
      return;
    }

    if (runtime.snoozedUntil) {
      const snoozedUntil = Date.parse(runtime.snoozedUntil);
      if (snoozedUntil > now.getTime()) {
        return;
      }
      const mealId = mealIdFromPromptKey(runtime.lastPromptKey);
      const mealEnabled = settings.meals.some(
        (meal) => meal.id === mealId && meal.enabled,
      );
      if (
        now.getTime() <= snoozedUntil + DUE_WINDOW_MS &&
        mealId &&
        mealEnabled
      ) {
        const activePrompt = {
          mealId,
          dueAt: new Date(snoozedUntil).toISOString(),
        };
        this.commit({
          ...candidateState,
          runtime: {
            activePrompt,
            ...(runtime.lastPromptKey
              ? { lastPromptKey: runtime.lastPromptKey }
              : {}),
          },
        });
        this.emitMealDue(activePrompt);
        return;
      }

      runtime = {
        ...(runtime.lastPromptKey
          ? { lastPromptKey: runtime.lastPromptKey }
          : {}),
      };
      runtimeChanged = true;
    }

    if (runtime.activePrompt) {
      const dueAt = Date.parse(runtime.activePrompt.dueAt);
      if (now.getTime() <= dueAt + DUE_WINDOW_MS) {
        return;
      }
      candidateState = this.commit({
        ...candidateState,
        runtime: {
          ...(runtime.lastPromptKey
            ? { lastPromptKey: runtime.lastPromptKey }
            : {}),
        },
      });
      runtime = candidateState.runtime;
    }
    const due = findDueMeal(settings.meals, now, settings.timezone, 45);
    if (!due || runtime.lastPromptKey === promptKey(due)) {
      if (timezoneChanged || runtimeChanged) {
        this.commit({ ...candidateState, runtime });
      }
      return;
    }

    const activePrompt: ActivePrompt = {
      mealId: due.mealId,
      dueAt: due.scheduledAt,
    };
    this.commit({
      ...candidateState,
      runtime: {
        activePrompt,
        lastPromptKey: promptKey(due),
      },
    });
    this.emitMealDue(activePrompt);
  }

  async snapshot(): Promise<CompanionSnapshot> {
    return cloneSnapshot(this.state);
  }

  async saveSettings(settings: CompanionSettings): Promise<CompanionSnapshot> {
    const parsedSettings = companionSettingsSchema.parse(settings);
    const pendingMealId =
      this.state.runtime.activePrompt?.mealId ??
      (this.state.runtime.snoozedUntil
        ? mealIdFromPromptKey(this.state.runtime.lastPromptKey)
        : undefined);
    const pendingMealEnabled =
      pendingMealId === undefined ||
      parsedSettings.meals.some(
        (meal) => meal.id === pendingMealId && meal.enabled,
      );
    const shouldClearPrompt = parsedSettings.quietMode || !pendingMealEnabled;
    const runtime = shouldClearPrompt
      ? {
          ...(this.state.runtime.lastPromptKey
            ? { lastPromptKey: this.state.runtime.lastPromptKey }
            : {}),
        }
      : this.state.runtime;
    return this.commit({ ...this.state, settings: parsedSettings, runtime });
  }

  async snoozeMeal(mealId: MealId, minutes = 10): Promise<CompanionSnapshot> {
    const activePrompt = this.requireActivePrompt(mealId);
    const now = this.now();
    const dueAt = new Date(
      now.getTime() + validatedSnoozeMinutes(minutes) * 60_000,
    ).toISOString();
    const lastPromptKey =
      this.state.runtime.lastPromptKey ??
      `${dateKeyAt(new Date(activePrompt.dueAt), this.state.settings.timezone)}:${mealId}`;
    return this.commit({
      ...this.state,
      runtime: {
        snoozedUntil: dueAt,
        lastPromptKey,
      },
    });
  }

  private historyEntry(
    mealId: MealId,
    dueAt: string,
    action: HistoryEntry["action"],
    choiceId?: string,
    mood?: RecommendationMood,
  ): HistoryEntry {
    const now = this.now();
    const normalizedChoiceId =
      choiceId === undefined ? undefined : choiceIdSchema.parse(choiceId);
    return {
      mealId,
      dateKey:
        dateKeyFromPromptKey(this.state.runtime.lastPromptKey, mealId) ??
        dateKeyAt(new Date(dueAt), this.state.settings.timezone),
      action,
      ...(normalizedChoiceId ? { choiceId: normalizedChoiceId } : {}),
      ...(mood ? { mood } : {}),
      at: now.toISOString(),
    };
  }

  private requireActivePrompt(mealId: MealId): ActivePrompt {
    const activePrompt = this.state.runtime.activePrompt;
    if (!activePrompt) {
      throw new Error("There is no active meal reminder");
    }
    if (activePrompt.mealId !== mealId) {
      throw new Error(`The active reminder is not for ${mealId}`);
    }
    return activePrompt;
  }

  private settleMeal(entry: HistoryEntry): CompanionSnapshot {
    const isDemo =
      this.state.runtime.lastPromptKey?.startsWith("demo:") === true;
    const lastPromptKey = isDemo
      ? this.state.runtime.lastPromptKey
      : `${entry.dateKey}:${entry.mealId}`;
    return this.commit({
      ...this.state,
      history: isDemo
        ? this.state.history
        : [...this.state.history, entry].slice(-120),
      runtime: {
        ...(lastPromptKey ? { lastPromptKey } : {}),
      },
    });
  }

  async skipMeal(mealId: MealId): Promise<CompanionSnapshot> {
    const activePrompt = this.requireActivePrompt(mealId);
    return this.settleMeal(
      this.historyEntry(mealId, activePrompt.dueAt, "skipped"),
    );
  }

  async completeMeal(
    mealId: MealId,
    choiceId?: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    const activePrompt = this.requireActivePrompt(mealId);
    return this.settleMeal(
      this.historyEntry(mealId, activePrompt.dueAt, "accepted", choiceId, mood),
    );
  }

  async dislikeSuggestion(
    mealId: MealId,
    choiceId: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    const activePrompt = this.requireActivePrompt(mealId);
    const entry = this.historyEntry(
      mealId,
      activePrompt.dueAt,
      "disliked",
      choiceId,
      mood,
    );
    const isDemo =
      this.state.runtime.lastPromptKey?.startsWith("demo:") === true;
    return this.commit({
      ...this.state,
      history: isDemo
        ? this.state.history
        : [...this.state.history, entry].slice(-120),
    });
  }

  async setQuietMode(quietMode: boolean): Promise<CompanionSnapshot> {
    return this.saveSettings({ ...this.state.settings, quietMode });
  }

  async triggerDemoReminder(mealId?: MealId): Promise<CompanionSnapshot> {
    const now = this.now();
    const selectedMeal =
      mealId ?? this.state.settings.meals.find((meal) => meal.enabled)?.id;
    if (!selectedMeal) {
      throw new Error("No configured meal is available for a demo reminder");
    }
    const activePrompt: ActivePrompt = {
      mealId: selectedMeal,
      dueAt: now.toISOString(),
    };
    const snapshot = this.commit({
      ...this.state,
      runtime: {
        activePrompt,
        lastPromptKey: `demo:${selectedMeal}:${now.getTime()}`,
      },
    });
    this.emitMealDue(activePrompt);
    return snapshot;
  }

  async setWindowMode(mode: WindowMode): Promise<void> {
    void mode;
  }

  async startWindowDrag(): Promise<void> {}

  async openNyxidAssistant(): Promise<void> {
    if (typeof window !== "undefined") {
      window.open(NYXID_ASSISTANT_URL, "_blank", "noopener,noreferrer");
    }
  }

  async nyxidStatus(): Promise<NyxIdView> {
    return { state: "unavailable" };
  }

  async startNyxidLogin(): Promise<NyxIdView> {
    return { state: "unavailable" };
  }

  async cancelNyxidLogin(): Promise<NyxIdView> {
    return { state: "unavailable" };
  }

  async refreshNyxidCapabilities(): Promise<NyxIdView> {
    return { state: "unavailable" };
  }

  async logoutNyxid(): Promise<NyxIdView> {
    return { state: "unavailable" };
  }

  async sendNyxIdChat(
    request: NyxIdChatRequest,
  ): Promise<NyxIdChatCompletedEvent> {
    void request;
    throw new NyxIdChatSendError(
      "rejected",
      "请使用桌面版连接 NyxID 后再发送消息。",
    );
  }

  async nyxIdChatHistory(conversationId: string): Promise<NyxIdChatHistory> {
    void conversationId;
    throw new Error("NyxID 对话需要在桌面应用中使用");
  }

  async recoverNyxIdChat(): Promise<NyxIdChatRecovery | null> {
    return null;
  }

  async stopNyxIdChat(conversationId: string): Promise<void> {
    void conversationId;
    throw new Error("NyxID 对话需要在桌面应用中使用");
  }

  async getLaunchAtLogin(): Promise<boolean> {
    try {
      return this.storage?.getItem(BROWSER_LAUNCH_AT_LOGIN_KEY) === "true";
    } catch {
      return false;
    }
  }

  async setLaunchAtLogin(enabled: boolean): Promise<boolean> {
    try {
      this.storage?.setItem(BROWSER_LAUNCH_AT_LOGIN_KEY, String(enabled));
    } catch {
      return false;
    }
    return enabled;
  }

  async onStateChanged(
    listener: (snapshot: CompanionSnapshot) => void,
  ): Promise<Unlisten> {
    this.stateListeners.add(listener);
    return () => {
      this.stateListeners.delete(listener);
    };
  }

  async onMealDue(listener: (prompt: ActivePrompt) => void): Promise<Unlisten> {
    this.mealListeners.add(listener);
    return () => {
      this.mealListeners.delete(listener);
    };
  }

  async onWindowDrag(
    listener: (event: WindowDragEvent) => void,
  ): Promise<Unlisten> {
    void listener;
    return () => undefined;
  }

  async onNyxidChanged(listener: (view: NyxIdView) => void): Promise<Unlisten> {
    void listener;
    return () => undefined;
  }

  async onNyxIdChatEvent(
    listener: (event: NyxIdChatEvent) => void,
  ): Promise<Unlisten> {
    void listener;
    return () => undefined;
  }

  dispose(): void {
    if (this.timer !== undefined) {
      this.clearTimer(this.timer);
      this.timer = undefined;
    }
    this.stateListeners.clear();
    this.mealListeners.clear();
  }
}
