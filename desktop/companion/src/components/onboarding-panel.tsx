import { useState } from "react";
import { ArrowRight, Check } from "lucide-react";

import type { CompanionSettings, MealId } from "../domain/companion";
import { MEAL_LABELS } from "../meal-copy";
import { BUDGET_OPTIONS, DIET_OPTIONS } from "../preference-options";
import { Mascot } from "./mascot";

interface OnboardingPanelProps {
  readonly initialSettings: CompanionSettings;
  readonly onComplete: (settings: CompanionSettings) => Promise<void>;
}

export function OnboardingPanel({
  initialSettings,
  onComplete,
}: OnboardingPanelProps) {
  const [step, setStep] = useState<1 | 2>(1);
  const [settings, setSettings] = useState(initialSettings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();

  function updateMeal(mealId: MealId, time: string) {
    setSettings((current) => ({
      ...current,
      meals: current.meals.map((meal) =>
        meal.id === mealId ? { ...meal, time } : meal,
      ),
    }));
  }

  function toggleDietary(id: string) {
    setSettings((current) => ({
      ...current,
      dietary: current.dietary.includes(id)
        ? current.dietary.filter((item) => item !== id)
        : [...current.dietary, id],
    }));
  }

  async function finish() {
    setSaving(true);
    setError(undefined);
    try {
      await onComplete({ ...settings, onboardingComplete: true });
    } catch (failure) {
      setError(
        failure instanceof Error ? failure.message : "设置没有保存，请再试一次",
      );
    } finally {
      setSaving(false);
    }
  }

  return (
    <main
      className="panel panel--onboarding"
      aria-labelledby="onboarding-title"
    >
      <div className="window-drag-strip" data-tauri-drag-region />
      <div className="onboarding-mascot" data-tauri-drag-region>
        <Mascot state={step === 1 ? "idle" : "hungry"} size={142} />
      </div>

      <div
        className="step-indicator"
        aria-label={`设置步骤 ${String(step)}，共 2 步`}
      >
        <span className={step >= 1 ? "is-active" : ""} />
        <span className={step >= 2 ? "is-active" : ""} />
      </div>

      {step === 1 ? (
        <section className="onboarding-step">
          <p className="eyebrow">初次见面</p>
          <h1 id="onboarding-title">给你的搭子起个名字</h1>
          <label className="field">
            <span>它叫什么</span>
            <input
              autoFocus
              name="companionName"
              autoComplete="off"
              maxLength={24}
              value={settings.companionName}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  companionName: event.target.value,
                }))
              }
              placeholder="Nyx"
            />
          </label>
          <label className="field">
            <span>怎么称呼你</span>
            <input
              name="userName"
              autoComplete="off"
              maxLength={24}
              value={settings.userName}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  userName: event.target.value,
                }))
              }
              placeholder="可不填"
            />
          </label>
          <button
            type="button"
            className="primary-button"
            disabled={!settings.companionName.trim()}
            onClick={() => setStep(2)}
          >
            记住饭点
            <ArrowRight aria-hidden="true" />
          </button>
        </section>
      ) : (
        <section className="onboarding-step">
          <p className="eyebrow">你的节奏</p>
          <h1 id="onboarding-title">一般几点吃饭？</h1>
          <div className="meal-time-list">
            {settings.meals.map((meal) => (
              <label className="meal-time-row" key={meal.id}>
                <span>{MEAL_LABELS[meal.id]}</span>
                <input
                  type="time"
                  name={`meal-${meal.id}`}
                  autoComplete="off"
                  aria-label={`${MEAL_LABELS[meal.id]}时间`}
                  value={meal.time}
                  onChange={(event) => updateMeal(meal.id, event.target.value)}
                />
              </label>
            ))}
          </div>

          <fieldset className="choice-group">
            <legend>平时偏好</legend>
            <div className="chip-row">
              {DIET_OPTIONS.map((option) => {
                const selected = settings.dietary.includes(option.id);
                return (
                  <button
                    type="button"
                    className={`choice-chip${selected ? " is-selected" : ""}`}
                    aria-pressed={selected}
                    key={option.id}
                    onClick={() => toggleDietary(option.id)}
                  >
                    {selected ? <Check aria-hidden="true" /> : null}
                    {option.label}
                  </button>
                );
              })}
            </div>
          </fieldset>

          <fieldset className="choice-group">
            <legend>一餐预算</legend>
            <div className="segmented-control">
              {BUDGET_OPTIONS.map((option) => (
                <button
                  type="button"
                  aria-pressed={settings.budget === option.id}
                  className={settings.budget === option.id ? "is-selected" : ""}
                  key={option.id}
                  onClick={() =>
                    setSettings((current) => ({
                      ...current,
                      budget: option.id,
                    }))
                  }
                >
                  {option.label}
                </button>
              ))}
            </div>
          </fieldset>

          {error ? (
            <p className="form-error" role="alert">
              {error}
            </p>
          ) : null}
          <div className="button-row">
            <button
              type="button"
              className="text-button"
              onClick={() => setStep(1)}
            >
              返回
            </button>
            <button
              type="button"
              className="primary-button"
              disabled={saving}
              onClick={() => void finish()}
            >
              {saving ? "正在记住" : "开始陪你"}
              <ArrowRight aria-hidden="true" />
            </button>
          </div>
        </section>
      )}
    </main>
  );
}
