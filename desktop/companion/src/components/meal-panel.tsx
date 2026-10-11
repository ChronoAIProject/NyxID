import { ArrowLeft, Check, Clock3, RefreshCw, Utensils } from "lucide-react";

import type { RecommendationMood } from "../domain/companion";
import { Mascot } from "./mascot";

export interface MealSuggestionView {
  readonly id: string;
  readonly name: string;
  readonly detail: string;
  readonly reason: string;
  readonly accent: "coral" | "mint" | "violet";
}

interface MealPanelProps {
  readonly companionName: string;
  readonly userName: string;
  readonly mealLabel: string;
  readonly mood?: RecommendationMood;
  readonly suggestions: ReadonlyArray<MealSuggestionView>;
  readonly canRefresh: boolean;
  readonly refreshing: boolean;
  readonly pending: boolean;
  readonly onMood: (mood: RecommendationMood) => void;
  readonly onChoose: (suggestion: MealSuggestionView) => void;
  readonly onDislike: (suggestion: MealSuggestionView) => void;
  readonly onRefresh: () => void;
  readonly onSnooze: () => void;
  readonly onSkip: () => void;
  readonly onBack: () => void;
}

const MOODS: ReadonlyArray<{ id: RecommendationMood; label: string }> = [
  { id: "light", label: "清淡" },
  { id: "balanced", label: "正常吃" },
  { id: "treat", label: "犒劳一下" },
];

export function MealPanel({
  companionName,
  userName,
  mealLabel,
  mood,
  suggestions,
  canRefresh,
  refreshing,
  pending,
  onMood,
  onChoose,
  onDislike,
  onRefresh,
  onSnooze,
  onSkip,
  onBack,
}: MealPanelProps) {
  const salutation = userName.trim() ? `${userName.trim()}，` : "";

  return (
    <main
      className="panel meal-panel"
      aria-labelledby="meal-title"
      aria-busy={pending}
    >
      <div className="panel-toolbar" data-window-drag-handle>
        <button
          type="button"
          className="bare-icon"
          aria-label="收起"
          onClick={onBack}
        >
          <ArrowLeft aria-hidden="true" />
        </button>
        <span>{companionName}</span>
        <span className="status-dot" aria-label="本地提醒已启用" />
      </div>

      <header className="meal-header">
        <Mascot state={mood ? "thinking" : "hungry"} size={126} />
        <div>
          <p className="eyebrow">{mealLabel}时间</p>
          <h1 id="meal-title">{salutation}今天想怎么吃？</h1>
        </div>
      </header>

      <div
        className="segmented-control segmented-control--mood"
        role="group"
        aria-label="选择今天的口味"
      >
        {MOODS.map((option) => (
          <button
            type="button"
            key={option.id}
            aria-pressed={mood === option.id}
            className={mood === option.id ? "is-selected" : ""}
            disabled={pending}
            onClick={() => onMood(option.id)}
          >
            {option.label}
          </button>
        ))}
      </div>

      {mood && suggestions.length > 0 ? (
        <section className="suggestion-section" aria-label="今日推荐">
          <div className="section-heading">
            <span>
              {suggestions.length > 0
                ? `给你留 ${String(suggestions.length)} 个`
                : "再想想别的"}
            </span>
            {canRefresh ? (
              <button
                type="button"
                className="inline-action"
                disabled={pending}
                onClick={onRefresh}
              >
                <RefreshCw
                  className={refreshing ? "is-spinning" : ""}
                  aria-hidden="true"
                />
                换一批
              </button>
            ) : null}
          </div>
          <div className="suggestion-list">
            {suggestions.map((suggestion, index) => (
              <article className="suggestion-row" key={suggestion.id}>
                <span
                  className={`suggestion-number suggestion-number--${suggestion.accent}`}
                >
                  {String(index + 1).padStart(2, "0")}
                </span>
                <div className="suggestion-copy">
                  <h2>{suggestion.name}</h2>
                  <p>{suggestion.detail}</p>
                  <span>{suggestion.reason}</span>
                </div>
                <div className="suggestion-actions">
                  <button
                    type="button"
                    className="choose-button"
                    disabled={pending}
                    onClick={() => onChoose(suggestion)}
                    aria-label={`就吃${suggestion.name}`}
                  >
                    <Check aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    className="dislike-button"
                    disabled={pending}
                    onClick={() => onDislike(suggestion)}
                    aria-label={`不喜欢${suggestion.name}`}
                  >
                    不想吃
                  </button>
                </div>
              </article>
            ))}
          </div>
        </section>
      ) : (
        <div className="meal-empty-state">
          <Utensils aria-hidden="true" />
          <p>
            {mood
              ? "这组偏好暂时没有合适选项，试试另一个口味或调整设置。"
              : "选一个今天的感觉，马上给你三种不纠结的答案。"}
          </p>
        </div>
      )}

      <footer className="meal-footer">
        <button
          type="button"
          className="secondary-button"
          disabled={pending}
          onClick={onSnooze}
        >
          <Clock3 aria-hidden="true" />
          10 分钟后
        </button>
        <button
          type="button"
          className="text-button"
          disabled={pending}
          onClick={onSkip}
        >
          今天跳过
        </button>
      </footer>
    </main>
  );
}
