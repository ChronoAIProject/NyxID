import { useEffect, useMemo, useRef, useState } from "react";

import { CelebrationPanel } from "./components/celebration-panel";
import { IdleView } from "./components/idle-view";
import { MealPanel, type MealSuggestionView } from "./components/meal-panel";
import { OnboardingPanel } from "./components/onboarding-panel";
import { SettingsPanel } from "./components/settings-panel";
import {
  dateKeyAt,
  dueLabel,
  nextMealOccurrence,
  recommendMeals,
  type CompanionSnapshot,
  type RecommendationMood,
} from "./domain";
import { MEAL_LABELS, recommendationCopy } from "./meal-copy";
import {
  createCompanionRuntime,
  isTauriHost,
  type CompanionRuntime,
  type NyxIdView,
  type Unlisten,
} from "./runtime";

type Screen = "idle" | "meal" | "settings" | "celebration";

interface Celebration {
  readonly choice: string;
}

interface CompanionAppProps {
  readonly runtime?: CompanionRuntime;
}

let sharedRuntime: CompanionRuntime | undefined;

function defaultRuntime(): CompanionRuntime {
  sharedRuntime ??= createCompanionRuntime();
  return sharedRuntime;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "发生了意外，请稍后再试";
}

function nextMealText(snapshot: CompanionSnapshot, now: Date): string {
  if (snapshot.runtime.snoozedUntil) {
    const snoozed = new Date(snapshot.runtime.snoozedUntil);
    if (snoozed.getTime() > now.getTime()) {
      const timing = dueLabel(snoozed, now, snapshot.settings.timezone);
      if (timing.kind === "in_minutes") {
        return `${String(timing.minutes)} 分钟后再来叫你`;
      }
      if (timing.kind === "in_hours") {
        return `${String(timing.hours)} 小时后再来叫你`;
      }
    }
  }

  const next = nextMealOccurrence(
    snapshot.settings.meals,
    now,
    snapshot.settings.timezone,
  );
  if (!next) {
    return "还没有开启饭点提醒";
  }

  const label = MEAL_LABELS[next.mealId];
  const timing = dueLabel(
    new Date(next.scheduledAt),
    now,
    snapshot.settings.timezone,
  );
  switch (timing.kind) {
    case "now":
      return `${label}就是现在`;
    case "overdue":
      return `${label}已经到了`;
    case "in_minutes":
      return `${String(timing.minutes)} 分钟后是${label}`;
    case "in_hours":
      return `${String(timing.hours)} 小时后是${label}`;
    case "tomorrow":
      return `明天 ${timing.time} 提醒${label}`;
    case "on_date":
      return `${timing.dateKey} ${timing.time} 提醒${label}`;
  }
}

function rotate<T>(values: ReadonlyArray<T>, offset: number): T[] {
  if (values.length === 0) {
    return [];
  }
  const normalized = offset % values.length;
  return [...values.slice(normalized), ...values.slice(0, normalized)];
}

function logicalPromptDateKey(snapshot: CompanionSnapshot): string | undefined {
  const prompt = snapshot.runtime.activePrompt;
  if (!prompt) {
    return undefined;
  }
  const match = /^(\d{4}-\d{2}-\d{2}):(breakfast|lunch|dinner)$/.exec(
    snapshot.runtime.lastPromptKey ?? "",
  );
  return match?.[2] === prompt.mealId
    ? match[1]
    : dateKeyAt(new Date(prompt.dueAt), snapshot.settings.timezone);
}

export function CompanionApp({ runtime: suppliedRuntime }: CompanionAppProps) {
  const runtime = useMemo(
    () => suppliedRuntime ?? defaultRuntime(),
    [suppliedRuntime],
  );
  const [snapshot, setSnapshot] = useState<CompanionSnapshot>();
  const [nyxIdView, setNyxIdView] = useState<NyxIdView>({
    state: "checking",
  });
  const [screen, setScreen] = useState<Screen>("idle");
  const [mood, setMood] = useState<RecommendationMood>();
  const [refreshRound, setRefreshRound] = useState(0);
  const [dislikedIds, setDislikedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [refreshing, setRefreshing] = useState(false);
  const [mealActionPending, setMealActionPending] = useState(false);
  const [launchAtLogin, setLaunchAtLoginState] = useState(false);
  const [celebration, setCelebration] = useState<Celebration>();
  const [status, setStatus] = useState<string>();
  const [fatalError, setFatalError] = useState<string>();
  const [now, setNow] = useState(() => new Date());
  const activePromptKey = useRef<string | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    const unlisteners: Unlisten[] = [];

    async function boot() {
      let receivedLiveState = false;
      try {
        const stateUnlisten = await runtime.onStateChanged((next) => {
          if (!alive) return;
          receivedLiveState = true;
          setSnapshot(next);
          if (next.runtime.activePrompt) {
            const promptKey = `${next.runtime.activePrompt.mealId}:${next.runtime.activePrompt.dueAt}`;
            if (activePromptKey.current !== promptKey) {
              activePromptKey.current = promptKey;
              setMood(undefined);
              setRefreshRound(0);
              setDislikedIds(new Set());
            }
            setScreen((current) => (current === "settings" ? current : "meal"));
          } else {
            activePromptKey.current = undefined;
            setScreen((current) => (current === "meal" ? "idle" : current));
          }
        });
        if (!alive) {
          await stateUnlisten();
          return;
        }
        unlisteners.push(stateUnlisten);

        const mealUnlisten = await runtime.onMealDue((prompt) => {
          if (alive) {
            const promptKey = `${prompt.mealId}:${prompt.dueAt}`;
            if (activePromptKey.current !== promptKey) {
              activePromptKey.current = promptKey;
              setMood(undefined);
              setRefreshRound(0);
              setDislikedIds(new Set());
            }
            setScreen((current) => (current === "settings" ? current : "meal"));
          }
        });
        if (!alive) {
          await mealUnlisten();
          return;
        }
        unlisteners.push(mealUnlisten);

        const initialSnapshot = await runtime.snapshot();
        let initialLaunchAtLogin = false;
        try {
          initialLaunchAtLogin = await runtime.getLaunchAtLogin();
        } catch {
          initialLaunchAtLogin = false;
        }
        if (!alive) return;
        setLaunchAtLoginState(initialLaunchAtLogin);
        if (!receivedLiveState) {
          setSnapshot(initialSnapshot);
          activePromptKey.current = initialSnapshot.runtime.activePrompt
            ? `${initialSnapshot.runtime.activePrompt.mealId}:${initialSnapshot.runtime.activePrompt.dueAt}`
            : undefined;
          setScreen(initialSnapshot.runtime.activePrompt ? "meal" : "idle");
        }
        setFatalError(undefined);
      } catch (error) {
        if (alive) setFatalError(errorMessage(error));
      }
    }

    void boot();
    return () => {
      alive = false;
      for (const unlisten of unlisteners) {
        void unlisten();
      }
    };
  }, [runtime]);

  useEffect(() => {
    let alive = true;
    let receivedLiveState = false;
    let unlisten: Unlisten | undefined;

    async function bootNyxId() {
      try {
        unlisten = await runtime.onNyxidChanged((next) => {
          if (!alive) return;
          receivedLiveState = true;
          setNyxIdView(next);
        });
        if (!alive) {
          await unlisten();
          return;
        }

        const initial = await runtime.nyxidStatus();
        if (alive && !receivedLiveState) {
          setNyxIdView(initial);
        }
      } catch (error) {
        if (alive) {
          setNyxIdView({
            state: "error",
            error: {
              code: "status_unavailable",
              message: errorMessage(error),
              retryable: true,
              retryAction: "refresh",
            },
          });
        }
      }
    }

    void bootNyxId();
    return () => {
      alive = false;
      if (unlisten) void unlisten();
    };
  }, [runtime]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!snapshot) return;
    const expanded = !snapshot.settings.onboardingComplete || screen !== "idle";
    void runtime
      .setWindowMode(expanded ? "expanded" : "compact")
      .catch((error) => {
        setStatus(`窗口调整失败：${errorMessage(error)}`);
      });
  }, [runtime, screen, snapshot]);

  useEffect(() => {
    if (!status) return;
    const timer = window.setTimeout(() => setStatus(undefined), 2_800);
    return () => window.clearTimeout(timer);
  }, [status]);

  useEffect(() => {
    if (!refreshing) return;
    const timer = window.setTimeout(() => setRefreshing(false), 360);
    return () => window.clearTimeout(timer);
  }, [refreshing]);

  useEffect(() => {
    if (screen !== "celebration") return;
    const timer = window.setTimeout(() => {
      setCelebration(undefined);
      setScreen("idle");
    }, 2_200);
    return () => window.clearTimeout(timer);
  }, [screen]);

  const activePrompt = snapshot?.runtime.activePrompt;
  const activeMeal = snapshot?.settings.meals.find(
    (meal) => meal.id === activePrompt?.mealId,
  );

  const recommendationView = useMemo<{
    suggestions: MealSuggestionView[];
    canRefresh: boolean;
  }>(() => {
    if (!snapshot || !activePrompt || !mood) {
      return { suggestions: [], canRefresh: false };
    }
    const currentDateKey =
      logicalPromptDateKey(snapshot) ??
      dateKeyAt(new Date(activePrompt.dueAt), snapshot.settings.timezone);
    const recommendationSet = recommendMeals({
      mealId: activePrompt.mealId,
      mood,
      settings: snapshot.settings,
      history: snapshot.history,
      dateKey: currentDateKey,
      limit: 6,
    });
    const ranked = [
      ...(recommendationSet.primary ? [recommendationSet.primary] : []),
      ...recommendationSet.alternatives,
    ];
    const rejected = new Set([
      ...dislikedIds,
      ...snapshot.history
        .filter(
          (entry) =>
            entry.action === "disliked" &&
            entry.mealId === activePrompt.mealId &&
            entry.dateKey === currentDateKey &&
            entry.choiceId,
        )
        .map((entry) => entry.choiceId!),
    ]);
    const remaining = ranked.filter(
      (recommendation) => !rejected.has(recommendation.id),
    );
    const suggestions = rotate(remaining, refreshRound)
      .slice(0, 3)
      .map((recommendation, index) => {
        const copy = recommendationCopy(recommendation, mood);
        const accents = ["coral", "mint", "violet"] as const;
        return {
          id: recommendation.id,
          ...copy,
          accent: accents[index] ?? "coral",
        };
      });
    return {
      suggestions,
      canRefresh: remaining.length > suggestions.length,
    };
  }, [activePrompt, dislikedIds, mood, refreshRound, snapshot]);

  async function triggerMealPicker() {
    try {
      setMood(undefined);
      setRefreshRound(0);
      setDislikedIds(new Set());
      const latest = await runtime.snapshot();
      const demoMealId =
        latest.settings.meals.find((meal) => meal.enabled)?.id ?? "lunch";
      const next = await runtime.triggerDemoReminder(demoMealId);
      setSnapshot(next);
      setScreen("meal");
    } catch (error) {
      setStatus(`暂时没能打开推荐：${errorMessage(error)}`);
    }
  }

  async function snooze() {
    if (!activePrompt || mealActionPending) return;
    setMealActionPending(true);
    try {
      const next = await runtime.snoozeMeal(activePrompt.mealId, 10);
      setSnapshot(next);
      setScreen("idle");
      setStatus("好，10 分钟后再来叫你");
    } catch (error) {
      setStatus(`稍后提醒失败：${errorMessage(error)}`);
    } finally {
      setMealActionPending(false);
    }
  }

  async function skip() {
    if (!activePrompt || mealActionPending) return;
    setMealActionPending(true);
    try {
      const next = await runtime.skipMeal(activePrompt.mealId);
      setSnapshot(next);
      setScreen("idle");
      setStatus("知道了，这一餐不再打扰你");
    } catch (error) {
      setStatus(`跳过失败：${errorMessage(error)}`);
    } finally {
      setMealActionPending(false);
    }
  }

  async function choose(suggestion: MealSuggestionView) {
    if (!activePrompt || mealActionPending) return;
    setMealActionPending(true);
    try {
      const next = await runtime.completeMeal(
        activePrompt.mealId,
        suggestion.id,
        mood,
      );
      setSnapshot(next);
      setCelebration({ choice: suggestion.name });
      setScreen("celebration");
    } catch (error) {
      setStatus(`还没能记住这个选择：${errorMessage(error)}`);
    } finally {
      setMealActionPending(false);
    }
  }

  async function dislike(suggestion: MealSuggestionView) {
    if (!activePrompt || mealActionPending) return;
    setMealActionPending(true);
    try {
      const next = await runtime.dislikeSuggestion(
        activePrompt.mealId,
        suggestion.id,
        mood,
      );
      setSnapshot(next);
      setDislikedIds((current) => new Set([...current, suggestion.id]));
      setRefreshRound((current) => current + 1);
      setStatus("记住了，以后少推荐这个");
    } catch (error) {
      setStatus(`反馈没有保存：${errorMessage(error)}`);
    } finally {
      setMealActionPending(false);
    }
  }

  if (fatalError) {
    return (
      <main className="app-error" role="alert">
        <div>
          <strong>Nyx 暂时没有醒来</strong>
          <p>{fatalError}</p>
          <button
            type="button"
            className="secondary-button"
            onClick={() => window.location.reload()}
          >
            再试一次
          </button>
        </div>
      </main>
    );
  }

  if (!snapshot) {
    return <main className="app-loading">Nyx 正在醒来…</main>;
  }

  let content;
  if (!snapshot.settings.onboardingComplete) {
    content = (
      <OnboardingPanel
        initialSettings={snapshot.settings}
        onComplete={async (settings) => {
          const next = await runtime.saveSettings(settings);
          setSnapshot(next);
          setScreen("idle");
        }}
      />
    );
  } else if (screen === "settings") {
    content = (
      <SettingsPanel
        settings={snapshot.settings}
        launchAtLogin={launchAtLogin}
        nyxIdView={nyxIdView}
        onClose={() =>
          setScreen(snapshot.runtime.activePrompt ? "meal" : "idle")
        }
        onSave={async (settings) => {
          const next = await runtime.saveSettings(settings);
          setSnapshot(next);
          setStatus("设置已经记住了");
          return next.settings;
        }}
        onSetLaunchAtLogin={async (enabled) => {
          const actual = await runtime.setLaunchAtLogin(enabled);
          setLaunchAtLoginState(actual);
        }}
        onDemoReminder={triggerMealPicker}
        onConnectNyxId={async () => {
          setNyxIdView(await runtime.startNyxidLogin());
        }}
        onCancelNyxId={async () => {
          setNyxIdView(await runtime.cancelNyxidLogin());
        }}
        onRefreshNyxId={async () => {
          setNyxIdView(await runtime.refreshNyxidCapabilities());
        }}
        onLogoutNyxId={async () => {
          setNyxIdView(await runtime.logoutNyxid());
        }}
        onOpenNyxIdAssistant={() => runtime.openNyxidAssistant()}
      />
    );
  } else if (screen === "celebration" && celebration) {
    content = (
      <CelebrationPanel
        companionName={snapshot.settings.companionName}
        choice={celebration.choice}
      />
    );
  } else if (screen === "meal" && activePrompt) {
    content = (
      <MealPanel
        companionName={snapshot.settings.companionName}
        userName={snapshot.settings.userName}
        mealLabel={
          MEAL_LABELS[activePrompt.mealId] ?? activeMeal?.label ?? "饭"
        }
        mood={mood}
        suggestions={recommendationView.suggestions}
        canRefresh={recommendationView.canRefresh}
        refreshing={refreshing}
        pending={mealActionPending}
        onMood={setMood}
        onChoose={(suggestion) => void choose(suggestion)}
        onDislike={(suggestion) => void dislike(suggestion)}
        onRefresh={() => {
          setRefreshing(true);
          setRefreshRound((current) => current + 1);
        }}
        onSnooze={() => void snooze()}
        onSkip={() => void skip()}
        onBack={() => setScreen("idle")}
      />
    );
  } else {
    content = (
      <IdleView
        name={snapshot.settings.companionName}
        nextMeal={nextMealText(snapshot, now)}
        quiet={snapshot.settings.quietMode}
        onOpen={() => {
          if (activePrompt) setScreen("meal");
          else void triggerMealPicker();
        }}
        onSettings={() => setScreen("settings")}
        onToggleQuiet={() => {
          void runtime
            .setQuietMode(!snapshot.settings.quietMode)
            .then((next) => {
              setSnapshot(next);
              setStatus(
                next.settings.quietMode ? "饭点提醒已暂停" : "饭点提醒恢复了",
              );
            })
            .catch((error) =>
              setStatus(`提醒设置失败：${errorMessage(error)}`),
            );
        }}
      />
    );
  }

  const expanded = !snapshot.settings.onboardingComplete || screen !== "idle";
  return (
    <div
      className={`app-frame${expanded ? " app-frame--expanded" : ""}${isTauriHost() ? "" : " app-frame--browser"}`}
    >
      {content}
      {status ? (
        <div className="live-status" role="status">
          {status}
        </div>
      ) : null}
    </div>
  );
}
