import { useState } from "react";
import {
  ArrowLeft,
  BellRing,
  Cloud,
  ExternalLink,
  LoaderCircle,
  Power,
  Save,
} from "lucide-react";

import type { CompanionSettings, MealId } from "../domain/companion";
import { MEAL_LABELS } from "../meal-copy";
import { BUDGET_OPTIONS, DIET_OPTIONS } from "../preference-options";

interface SettingsPanelProps {
  readonly settings: CompanionSettings;
  readonly launchAtLogin: boolean;
  readonly onClose: () => void;
  readonly onSave: (settings: CompanionSettings) => Promise<CompanionSettings>;
  readonly onSetLaunchAtLogin: (enabled: boolean) => Promise<void>;
  readonly onDemoReminder: () => Promise<void>;
  readonly onOpenNyxid: () => Promise<void>;
}

function SwitchControl({
  checked,
  label,
  onChange,
}: {
  readonly checked: boolean;
  readonly label: string;
  readonly onChange: (checked: boolean) => void;
}) {
  return (
    <button
      type="button"
      className={`switch-control${checked ? " is-on" : ""}`}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function SettingsPanel({
  settings,
  launchAtLogin,
  onClose,
  onSave,
  onSetLaunchAtLogin,
  onDemoReminder,
  onOpenNyxid,
}: SettingsPanelProps) {
  const [draftOverrides, setDraftOverrides] = useState<
    Partial<CompanionSettings>
  >({});
  const [avoidTextOverride, setAvoidTextOverride] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [runningAction, setRunningAction] = useState(false);
  const [error, setError] = useState<string>();
  const draft: CompanionSettings = { ...settings, ...draftOverrides };
  const avoidText = avoidTextOverride ?? settings.avoid.join("、");
  const interactionPending = saving || runningAction;

  const normalizedAvoid = avoidText
    .split(/[，,、\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
  const normalizedAvoidKeys = normalizedAvoid.map((item) =>
    item.normalize("NFKC").toLocaleLowerCase(),
  );
  const avoidError =
    normalizedAvoid.length > 20
      ? "不想吃的食材最多填写 20 项"
      : new Set(normalizedAvoidKeys).size !== normalizedAvoidKeys.length
        ? "不想吃的食材里有重复项"
        : undefined;
  const currentDraft: CompanionSettings = { ...draft, avoid: normalizedAvoid };
  const dirty = JSON.stringify(currentDraft) !== JSON.stringify(settings);

  function close() {
    if (!dirty || window.confirm("放弃尚未保存的修改？")) {
      onClose();
    }
  }

  function updateMeal(
    mealId: MealId,
    change: Partial<CompanionSettings["meals"][number]>,
  ) {
    setDraftOverrides((current) => ({
      ...current,
      meals: draft.meals.map((meal) =>
        meal.id === mealId ? { ...meal, ...change } : meal,
      ),
    }));
  }

  function toggleDietary(id: string) {
    setDraftOverrides((current) => ({
      ...current,
      dietary: draft.dietary.includes(id)
        ? draft.dietary.filter((item) => item !== id)
        : [...draft.dietary, id],
    }));
  }

  function acceptSavedSettings() {
    setDraftOverrides({});
    setAvoidTextOverride(undefined);
  }

  async function save() {
    if (avoidError) {
      setError(avoidError);
      return;
    }
    setSaving(true);
    setError(undefined);
    try {
      await onSave(currentDraft);
      acceptSavedSettings();
    } catch (failure) {
      setError(
        failure instanceof Error ? failure.message : "设置没有保存，请再试一次",
      );
    } finally {
      setSaving(false);
    }
  }

  async function updateLaunch(enabled: boolean) {
    setLaunching(true);
    setError(undefined);
    try {
      await onSetLaunchAtLogin(enabled);
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "开机启动设置失败，请再试一次",
      );
    } finally {
      setLaunching(false);
    }
  }

  async function runAction(action: () => Promise<void>) {
    setRunningAction(true);
    setError(undefined);
    try {
      await action();
    } catch (failure) {
      setError(
        failure instanceof Error ? failure.message : "操作失败，请再试一次",
      );
    } finally {
      setRunningAction(false);
    }
  }

  async function previewReminder() {
    if (avoidError) {
      throw new Error(avoidError);
    }
    if (dirty) {
      await onSave(currentDraft);
      acceptSavedSettings();
    }
    await onDemoReminder();
  }

  return (
    <main
      className="panel settings-panel"
      aria-busy={interactionPending}
      aria-labelledby="settings-title"
    >
      <div className="panel-toolbar" data-tauri-drag-region>
        <button
          type="button"
          className="bare-icon"
          aria-label="返回"
          disabled={interactionPending}
          onClick={close}
        >
          <ArrowLeft aria-hidden="true" />
        </button>
        <h1 id="settings-title">陪伴设置</h1>
        <button
          type="button"
          className="save-icon"
          aria-label="保存设置"
          disabled={interactionPending || !dirty || Boolean(avoidError)}
          onClick={() => void save()}
        >
          {saving ? <LoaderCircle className="is-spinning" /> : <Save />}
        </button>
      </div>

      <fieldset
        className="settings-scroll settings-fields"
        disabled={interactionPending}
      >
        {avoidError || error ? (
          <p className="form-error form-error--settings" role="alert">
            {avoidError ?? error}
          </p>
        ) : null}
        <section className="settings-section">
          <h2>称呼</h2>
          <div className="two-column-fields">
            <label className="field">
              <span>搭子名字</span>
              <input
                name="companionName"
                autoComplete="off"
                maxLength={24}
                value={draft.companionName}
                onChange={(event) =>
                  setDraftOverrides((current) => ({
                    ...current,
                    companionName: event.target.value,
                  }))
                }
              />
            </label>
            <label className="field">
              <span>你的名字</span>
              <input
                name="userName"
                autoComplete="off"
                maxLength={24}
                value={draft.userName}
                onChange={(event) =>
                  setDraftOverrides((current) => ({
                    ...current,
                    userName: event.target.value,
                  }))
                }
              />
            </label>
          </div>
        </section>

        <section className="settings-section">
          <h2>饭点</h2>
          <div className="settings-meals">
            {draft.meals.map((meal) => (
              <div className="settings-meal-row" key={meal.id}>
                <SwitchControl
                  checked={meal.enabled}
                  label={`${MEAL_LABELS[meal.id]}提醒`}
                  onChange={(enabled) => updateMeal(meal.id, { enabled })}
                />
                <span>{MEAL_LABELS[meal.id]}</span>
                <input
                  type="time"
                  name={`meal-${meal.id}`}
                  autoComplete="off"
                  aria-label={`${MEAL_LABELS[meal.id]}时间`}
                  value={meal.time}
                  disabled={!meal.enabled}
                  onChange={(event) =>
                    updateMeal(meal.id, { time: event.target.value })
                  }
                />
              </div>
            ))}
          </div>
          <button
            type="button"
            className="secondary-button secondary-button--full"
            disabled={runningAction || saving}
            onClick={() => void runAction(previewReminder)}
          >
            <BellRing aria-hidden="true" />
            现在试一次提醒
          </button>
        </section>

        <section className="settings-section">
          <h2>口味</h2>
          <div className="chip-row">
            {DIET_OPTIONS.map((option) => {
              const selected = draft.dietary.includes(option.id);
              return (
                <button
                  type="button"
                  className={`choice-chip${selected ? " is-selected" : ""}`}
                  aria-pressed={selected}
                  key={option.id}
                  onClick={() => toggleDietary(option.id)}
                >
                  {option.label}
                </button>
              );
            })}
          </div>
          <label className="field field--spaced">
            <span>不想吃的食材</span>
            <input
              name="avoidIngredients"
              autoComplete="off"
              value={avoidText}
              onChange={(event) => setAvoidTextOverride(event.target.value)}
              placeholder="香菜、花生、内脏"
            />
          </label>
          <div className="segmented-control" role="group" aria-label="一餐预算">
            {BUDGET_OPTIONS.map((option) => (
              <button
                type="button"
                aria-pressed={draft.budget === option.id}
                className={draft.budget === option.id ? "is-selected" : ""}
                key={option.id}
                onClick={() =>
                  setDraftOverrides((current) => ({
                    ...current,
                    budget: option.id,
                  }))
                }
              >
                {option.label}
              </button>
            ))}
          </div>
        </section>

        <section className="settings-section settings-section--rows">
          <div className="setting-row">
            <div>
              <strong>暂停饭点提醒</strong>
              <span>保持安静，直到你手动恢复</span>
            </div>
            <SwitchControl
              checked={draft.quietMode}
              label="暂停饭点提醒"
              onChange={(quietMode) =>
                setDraftOverrides((current) => ({ ...current, quietMode }))
              }
            />
          </div>
          <div className="setting-row">
            <div>
              <strong>登录时启动</strong>
              <span>开机后在右下角等你</span>
            </div>
            {launching ? (
              <LoaderCircle
                className="setting-loader is-spinning"
                aria-label="正在更新"
              />
            ) : (
              <SwitchControl
                checked={launchAtLogin}
                label="登录时启动"
                onChange={(enabled) => void updateLaunch(enabled)}
              />
            )}
          </div>
        </section>

        <section className="settings-section nyxid-row">
          <div className="nyxid-mark" aria-hidden="true">
            <Cloud />
          </div>
          <div>
            <h2>NyxID</h2>
            <p>打开你的 Assistant、服务和授权</p>
          </div>
          <button
            type="button"
            className="inline-action"
            disabled={runningAction}
            onClick={() => void runAction(onOpenNyxid)}
          >
            打开
            <ExternalLink aria-hidden="true" />
          </button>
        </section>

        <section className="privacy-note">
          <Power aria-hidden="true" />
          <p>饭点和口味只保存在这台电脑。外部服务需要在 NyxID 中单独授权。</p>
        </section>
      </fieldset>
    </main>
  );
}
