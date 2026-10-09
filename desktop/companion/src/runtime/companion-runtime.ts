import type {
  ActivePrompt,
  CompanionSettings,
  CompanionSnapshot,
  MealId,
  RecommendationMood,
} from "../domain/companion";
import type { NyxIdView } from "./nyxid";

export type WindowMode = "compact" | "expanded";
export type Unlisten = () => void | Promise<void>;

export interface CompanionRuntime {
  snapshot(): Promise<CompanionSnapshot>;
  saveSettings(settings: CompanionSettings): Promise<CompanionSnapshot>;
  snoozeMeal(mealId: MealId, minutes?: number): Promise<CompanionSnapshot>;
  skipMeal(mealId: MealId): Promise<CompanionSnapshot>;
  completeMeal(
    mealId: MealId,
    choiceId?: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot>;
  dislikeSuggestion(
    mealId: MealId,
    choiceId: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot>;
  setQuietMode(quietMode: boolean): Promise<CompanionSnapshot>;
  triggerDemoReminder(mealId?: MealId): Promise<CompanionSnapshot>;
  setWindowMode(mode: WindowMode): Promise<void>;
  openNyxidAssistant(): Promise<void>;
  nyxidStatus(): Promise<NyxIdView>;
  startNyxidLogin(): Promise<NyxIdView>;
  cancelNyxidLogin(): Promise<NyxIdView>;
  refreshNyxidCapabilities(): Promise<NyxIdView>;
  logoutNyxid(): Promise<NyxIdView>;
  getLaunchAtLogin(): Promise<boolean>;
  setLaunchAtLogin(enabled: boolean): Promise<boolean>;
  onStateChanged(
    listener: (snapshot: CompanionSnapshot) => void,
  ): Promise<Unlisten>;
  onMealDue(listener: (prompt: ActivePrompt) => void): Promise<Unlisten>;
  onNyxidChanged(listener: (view: NyxIdView) => void): Promise<Unlisten>;
  dispose(): void | Promise<void>;
}
