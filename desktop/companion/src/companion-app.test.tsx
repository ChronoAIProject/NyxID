import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CompanionApp } from "./companion-app";
import {
  createDefaultCompanionSnapshot,
  parseCompanionSnapshot,
  type ActivePrompt,
  type CompanionSettings,
  type CompanionSnapshot,
  type MealId,
  type RecommendationMood,
} from "./domain";
import type { CompanionRuntime, Unlisten, WindowMode } from "./runtime";

function clone(snapshot: CompanionSnapshot): CompanionSnapshot {
  return parseCompanionSnapshot(snapshot, snapshot.settings.timezone);
}

function onboardedSnapshot(): CompanionSnapshot {
  const snapshot = createDefaultCompanionSnapshot("UTC");
  snapshot.settings = {
    ...snapshot.settings,
    companionName: "Nyx",
    userName: "小葵",
    onboardingComplete: true,
  };
  return snapshot;
}

class TestRuntime implements CompanionRuntime {
  private current: CompanionSnapshot;
  private readonly stateListeners = new Set<
    (snapshot: CompanionSnapshot) => void
  >();
  private readonly mealListeners = new Set<(prompt: ActivePrompt) => void>();

  readonly saveSettingsCall = vi.fn();
  readonly dislikeCall = vi.fn();
  readonly completeCall = vi.fn();
  readonly snoozeCall = vi.fn();
  readonly skipCall = vi.fn();
  readonly setWindowModeCall = vi.fn();
  readonly openNyxidAssistantCall = vi.fn();
  readonly triggerDemoCall = vi.fn();
  readonly snapshotCall = vi.fn();
  private readonly launchAtLoginResult: Promise<boolean>;
  private readonly saveSettingsGate: Promise<void>;

  constructor(
    snapshot: CompanionSnapshot,
    launchAtLoginResult: Promise<boolean> = Promise.resolve(false),
    saveSettingsGate: Promise<void> = Promise.resolve(),
  ) {
    this.current = clone(snapshot);
    this.launchAtLoginResult = launchAtLoginResult;
    this.saveSettingsGate = saveSettingsGate;
  }

  private commit(snapshot: CompanionSnapshot): CompanionSnapshot {
    this.current = clone(snapshot);
    for (const listener of this.stateListeners) {
      listener(clone(this.current));
    }
    return clone(this.current);
  }

  async snapshot(): Promise<CompanionSnapshot> {
    this.snapshotCall();
    return clone(this.current);
  }

  emitSnapshot(snapshot: CompanionSnapshot): void {
    this.commit(snapshot);
  }

  emitMealDue(prompt: ActivePrompt): void {
    for (const listener of this.mealListeners) {
      listener(prompt);
    }
  }

  async saveSettings(settings: CompanionSettings): Promise<CompanionSnapshot> {
    this.saveSettingsCall(settings);
    await this.saveSettingsGate;
    return this.commit({ ...this.current, settings });
  }

  async snoozeMeal(mealId: MealId, minutes = 10): Promise<CompanionSnapshot> {
    this.snoozeCall(mealId, minutes);
    return this.commit({
      ...this.current,
      runtime: {
        snoozedUntil: "2026-10-09T12:41:00.000Z",
        ...(this.current.runtime.lastPromptKey
          ? { lastPromptKey: this.current.runtime.lastPromptKey }
          : {}),
      },
    });
  }

  async skipMeal(mealId: MealId): Promise<CompanionSnapshot> {
    this.skipCall(mealId);
    return this.commit({ ...this.current, runtime: {} });
  }

  async completeMeal(
    mealId: MealId,
    choiceId?: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    this.completeCall(mealId, choiceId, mood);
    return this.commit({ ...this.current, runtime: {} });
  }

  async dislikeSuggestion(
    mealId: MealId,
    choiceId: string,
    mood?: RecommendationMood,
  ): Promise<CompanionSnapshot> {
    this.dislikeCall(mealId, choiceId, mood);
    return this.commit(this.current);
  }

  async setQuietMode(quietMode: boolean): Promise<CompanionSnapshot> {
    return this.commit({
      ...this.current,
      settings: { ...this.current.settings, quietMode },
    });
  }

  async triggerDemoReminder(
    mealId: MealId = "lunch",
  ): Promise<CompanionSnapshot> {
    this.triggerDemoCall(mealId);
    const prompt: ActivePrompt = {
      mealId,
      dueAt: "2026-10-09T12:31:00.000Z",
    };
    const next = this.commit({
      ...this.current,
      runtime: {
        activePrompt: prompt,
        lastPromptKey: `demo:${mealId}:1`,
      },
    });
    for (const listener of this.mealListeners) {
      listener(prompt);
    }
    return next;
  }

  async setWindowMode(mode: WindowMode): Promise<void> {
    this.setWindowModeCall(mode);
  }

  async openNyxidAssistant(): Promise<void> {
    this.openNyxidAssistantCall();
  }

  async getLaunchAtLogin(): Promise<boolean> {
    return this.launchAtLoginResult;
  }

  async setLaunchAtLogin(enabled: boolean): Promise<boolean> {
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

  dispose(): void {
    this.stateListeners.clear();
    this.mealListeners.clear();
  }
}

describe("CompanionApp", () => {
  it("finishes first-run setup and collapses into the desktop companion", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(createDefaultCompanionSnapshot("UTC"));

    render(<CompanionApp runtime={runtime} />);

    expect(
      await screen.findByRole("heading", { name: "给你的搭子起个名字" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "记住饭点" }));
    expect(
      screen.getByRole("heading", { name: "一般几点吃饭？" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "开始陪你" }));

    expect(
      await screen.findByRole("button", { name: "让 Nyx 帮我选吃的" }),
    ).toBeInTheDocument();
    expect(runtime.saveSettingsCall).toHaveBeenCalledWith(
      expect.objectContaining({ onboardingComplete: true }),
    );
    await waitFor(() => {
      expect(runtime.setWindowModeCall).toHaveBeenLastCalledWith("compact");
    });
  });

  it("turns a due meal into explainable choices and remembers feedback", async () => {
    const user = userEvent.setup();
    const snapshot = onboardedSnapshot();
    snapshot.runtime = {
      activePrompt: {
        mealId: "lunch",
        dueAt: "2026-10-09T12:30:00.000Z",
      },
      lastPromptKey: "2026-10-09:lunch",
    };
    const runtime = new TestRuntime(snapshot);

    render(<CompanionApp runtime={runtime} />);

    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "正常吃" }));
    expect(await screen.findAllByRole("article")).toHaveLength(3);

    const dislikedButton = screen.getAllByRole("button", {
      name: /^不喜欢/,
    })[0]!;
    const dislikedLabel = dislikedButton.getAttribute("aria-label")!;
    await user.click(dislikedButton);
    await waitFor(() => expect(runtime.dislikeCall).toHaveBeenCalledTimes(1));
    expect(screen.getByText("记住了，以后少推荐这个")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: dislikedLabel }),
    ).not.toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: /^就吃/ })[0]!);
    await waitFor(() => expect(runtime.completeCall).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("决定好了")).toBeInTheDocument();
    expect(screen.getByText(/下次会更懂你的口味/)).toBeInTheDocument();
  });

  it("opens NyxID from settings and can start a demo reminder", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());
    const openSpy = vi.spyOn(runtime, "openNyxidAssistant");

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));

    expect(
      screen.getByRole("heading", { name: "陪伴设置" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "打开" }));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledTimes(1);
    });

    await user.click(screen.getByRole("button", { name: "现在试一次提醒" }));
    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    expect(runtime.triggerDemoCall).toHaveBeenCalledTimes(1);
  });

  it("keeps a dirty settings draft open when a meal becomes due", async () => {
    const user = userEvent.setup();
    const initial = onboardedSnapshot();
    const runtime = new TestRuntime(initial);

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    const companionName = screen.getByRole("textbox", { name: "搭子名字" });
    await user.clear(companionName);
    await user.type(companionName, "Momo");

    const due = clone(initial);
    const prompt: ActivePrompt = {
      mealId: "dinner",
      dueAt: "2026-10-09T18:30:00.000Z",
    };
    due.runtime = {
      activePrompt: prompt,
      lastPromptKey: "2026-10-09:dinner",
    };
    act(() => {
      runtime.emitSnapshot(due);
      runtime.emitMealDue(prompt);
    });

    expect(
      screen.getByRole("heading", { name: "陪伴设置" }),
    ).toBeInTheDocument();
    expect(companionName).toHaveValue("Momo");
    expect(
      screen.queryByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(runtime.saveSettingsCall).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "返回" }));

    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    expect(screen.getByText("晚餐时间")).toBeInTheDocument();
  });

  it("merges untouched external settings without losing a local name edit", async () => {
    const user = userEvent.setup();
    const initial = onboardedSnapshot();
    const runtime = new TestRuntime(initial);

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    const companionName = screen.getByRole("textbox", { name: "搭子名字" });
    await user.clear(companionName);
    await user.type(companionName, "Momo");

    const trayUpdate = clone(initial);
    trayUpdate.settings = {
      ...trayUpdate.settings,
      quietMode: true,
      timezone: "Asia/Shanghai",
    };
    act(() => runtime.emitSnapshot(trayUpdate));

    expect(companionName).toHaveValue("Momo");
    expect(
      screen.getByRole("switch", { name: "暂停饭点提醒" }),
    ).toHaveAttribute("aria-checked", "true");

    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(runtime.saveSettingsCall).toHaveBeenCalledWith(
        expect.objectContaining({
          companionName: "Momo",
          quietMode: true,
          timezone: "Asia/Shanghai",
        }),
      );
    });
  });

  it("keeps a local quiet-mode edit while merging an external timezone", async () => {
    const user = userEvent.setup();
    const initial = onboardedSnapshot();
    const runtime = new TestRuntime(initial);

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    const quietMode = screen.getByRole("switch", { name: "暂停饭点提醒" });
    await user.click(quietMode);

    const externalUpdate = clone(initial);
    externalUpdate.settings = {
      ...externalUpdate.settings,
      quietMode: false,
      timezone: "Asia/Shanghai",
    };
    act(() => runtime.emitSnapshot(externalUpdate));

    expect(quietMode).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(runtime.saveSettingsCall).toHaveBeenCalledWith(
        expect.objectContaining({
          quietMode: true,
          timezone: "Asia/Shanghai",
        }),
      );
    });
  });

  it("locks the settings draft while a save is pending", async () => {
    const user = userEvent.setup();
    const initial = onboardedSnapshot();
    let finishSave: (() => void) | undefined;
    const saveGate = new Promise<void>((resolve) => {
      finishSave = resolve;
    });
    const runtime = new TestRuntime(initial, Promise.resolve(false), saveGate);

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    const companionName = screen.getByRole("textbox", { name: "搭子名字" });
    await user.clear(companionName);
    await user.type(companionName, "Momo");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => expect(runtime.saveSettingsCall).toHaveBeenCalled());
    expect(companionName).toBeDisabled();
    expect(screen.getByRole("switch", { name: "暂停饭点提醒" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "返回" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存设置" })).toBeDisabled();

    await user.type(companionName, " should-not-appear");
    expect(companionName).toHaveValue("Momo");

    finishSave?.();

    await waitFor(() => expect(companionName).toBeEnabled());
    expect(companionName).toHaveValue("Momo");
    expect(runtime.saveSettingsCall).toHaveBeenCalledWith(
      expect.objectContaining({ companionName: "Momo" }),
    );
  });

  it("does not overwrite a live meal event with a stale boot snapshot", async () => {
    let finishLaunchRead: ((enabled: boolean) => void) | undefined;
    const launchRead = new Promise<boolean>((resolve) => {
      finishLaunchRead = resolve;
    });
    const runtime = new TestRuntime(onboardedSnapshot(), launchRead);

    render(<CompanionApp runtime={runtime} />);
    await waitFor(() => expect(runtime.snapshotCall).toHaveBeenCalledTimes(1));

    const due = onboardedSnapshot();
    due.runtime = {
      activePrompt: {
        mealId: "dinner",
        dueAt: "2026-10-09T18:30:00.000Z",
      },
      lastPromptKey: "2026-10-09:dinner",
    };
    await act(async () => {
      runtime.emitSnapshot(due);
      finishLaunchRead?.(false);
    });

    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    expect(screen.getByText("晚餐时间")).toBeInTheDocument();
  });

  it("does not dispose a caller-owned runtime during StrictMode mount or unmount", async () => {
    const runtime = new TestRuntime(onboardedSnapshot());
    const dispose = vi.spyOn(runtime, "dispose");

    const { unmount } = render(
      <StrictMode>
        <CompanionApp runtime={runtime} />
      </StrictMode>,
    );

    expect(
      await screen.findByRole("button", { name: "让 Nyx 帮我选吃的" }),
    ).toBeInTheDocument();
    expect(dispose).not.toHaveBeenCalled();

    unmount();

    expect(dispose).not.toHaveBeenCalled();
  });

  it("keeps dislikes tied to the original meal date after a snooze crosses midnight", async () => {
    const user = userEvent.setup();
    const initial = onboardedSnapshot();
    initial.settings = {
      ...initial.settings,
      budget: "low",
      dietary: ["vegan"],
      avoid: ["wheat"],
    };
    initial.runtime = {
      activePrompt: {
        mealId: "dinner",
        dueAt: "2026-10-09T23:55:00.000Z",
      },
      lastPromptKey: "2026-10-09:dinner",
    };
    const runtime = new TestRuntime(initial);

    render(<CompanionApp runtime={runtime} />);
    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "正常吃" }));

    expect(await screen.findAllByRole("article")).toHaveLength(1);
    const rejectedChoiceLabel = screen
      .getByRole("button", { name: /^就吃/ })
      .getAttribute("aria-label")!;
    await user.click(screen.getByRole("button", { name: /^不喜欢/ }));
    await waitFor(() => expect(runtime.dislikeCall).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "10 分钟后" }));
    await waitFor(() => expect(runtime.snoozeCall).toHaveBeenCalledTimes(1));

    const resumed = clone(initial);
    resumed.history = [
      {
        mealId: "dinner",
        dateKey: "2026-10-09",
        action: "disliked",
        choiceId: "lemon-lentil-soup",
        mood: "balanced",
        at: "2026-10-09T23:56:00.000Z",
      },
    ];
    resumed.runtime = {
      activePrompt: {
        mealId: "dinner",
        dueAt: "2026-10-10T00:06:00.000Z",
      },
      lastPromptKey: "2026-10-09:dinner",
    };
    act(() => runtime.emitSnapshot(resumed));

    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "正常吃" }));

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: rejectedChoiceLabel }),
      ).not.toBeInTheDocument();
    });
    expect(
      screen.getByText("这组偏好暂时没有合适选项，试试另一个口味或调整设置。"),
    ).toBeInTheDocument();
  });
});
