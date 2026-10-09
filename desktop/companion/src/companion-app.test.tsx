import { StrictMode } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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
import {
  NyxIdChatSendError,
  type CompanionRuntime,
  type NyxIdChatCompletedEvent,
  type NyxIdChatEvent,
  type NyxIdChatHistory,
  type NyxIdChatRecovery,
  type NyxIdChatRequest,
  type NyxIdView,
  type Unlisten,
  type WindowDragEvent,
  type WindowMode,
} from "./runtime";

const CHAT_CONVERSATION_ID = "nyxa-0123456789abcdef0123456789abcdef";
const CHAT_RECOVERY_REQUEST_ID = "123e4567-e89b-42d3-a456-426614174000";

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

function connectedNyxIdView(): NyxIdView {
  return {
    state: "connected",
    user: {
      id: "user-1",
      email: "xiaokui@example.com",
      displayName: "小葵",
    },
    capabilities: {
      enabledCount: 2,
      disabledCount: 1,
      attentionCount: 1,
      checkedAt: "2026-10-09T10:00:00Z",
      services: [
        {
          id: "service-1",
          slug: "api-github",
          label: "GitHub",
          state: "enabled",
        },
        {
          id: "service-2",
          slug: "api-google",
          label: "Google",
          state: "attention",
        },
        {
          id: "service-3",
          slug: "llm-openai",
          label: "OpenAI",
          state: "disabled",
        },
      ],
    },
  };
}

class TestRuntime implements CompanionRuntime {
  private current: CompanionSnapshot;
  private readonly stateListeners = new Set<
    (snapshot: CompanionSnapshot) => void
  >();
  private readonly mealListeners = new Set<(prompt: ActivePrompt) => void>();
  private readonly windowDragListeners = new Set<
    (event: WindowDragEvent) => void
  >();
  private readonly nyxIdListeners = new Set<(view: NyxIdView) => void>();
  private readonly chatListeners = new Set<(event: NyxIdChatEvent) => void>();

  readonly saveSettingsCall = vi.fn();
  readonly dislikeCall = vi.fn();
  readonly completeCall = vi.fn();
  readonly snoozeCall = vi.fn();
  readonly skipCall = vi.fn();
  readonly setWindowModeCall = vi.fn();
  readonly startWindowDragCall = vi.fn();
  readonly openNyxidAssistantCall = vi.fn();
  readonly startNyxIdLoginCall = vi.fn();
  readonly cancelNyxIdLoginCall = vi.fn();
  readonly refreshNyxIdCall = vi.fn();
  readonly logoutNyxIdCall = vi.fn();
  readonly nyxIdStatusCall = vi.fn();
  readonly triggerDemoCall = vi.fn();
  readonly snapshotCall = vi.fn();
  readonly sendNyxIdChatCall = vi.fn();
  readonly recoverNyxIdChatCall = vi.fn(
    async (): Promise<NyxIdChatRecovery | null> => null,
  );
  readonly nyxIdChatHistoryCall = vi.fn();
  readonly stopNyxIdChatCall = vi.fn();
  private readonly launchAtLoginResult: Promise<boolean>;
  private readonly saveSettingsGate: Promise<void>;
  private readonly nyxIdStatusResult: Promise<NyxIdView>;
  private currentNyxId: NyxIdView = { state: "signed_out" };

  constructor(
    snapshot: CompanionSnapshot,
    launchAtLoginResult: Promise<boolean> = Promise.resolve(false),
    saveSettingsGate: Promise<void> = Promise.resolve(),
    nyxIdStatusResult: Promise<NyxIdView> = Promise.resolve({
      state: "signed_out",
    }),
  ) {
    this.current = clone(snapshot);
    this.launchAtLoginResult = launchAtLoginResult;
    this.saveSettingsGate = saveSettingsGate;
    this.nyxIdStatusResult = nyxIdStatusResult;
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

  emitWindowDrag(event: WindowDragEvent): void {
    for (const listener of this.windowDragListeners) {
      listener(event);
    }
  }

  emitNyxIdView(view: NyxIdView): void {
    this.currentNyxId = view;
    for (const listener of this.nyxIdListeners) {
      listener(view);
    }
  }

  emitNyxIdChatEvent(event: NyxIdChatEvent): void {
    for (const listener of this.chatListeners) {
      listener(event);
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

  async startWindowDrag(): Promise<void> {
    this.startWindowDragCall();
  }

  async openNyxidAssistant(): Promise<void> {
    this.openNyxidAssistantCall();
  }

  async nyxidStatus(): Promise<NyxIdView> {
    this.nyxIdStatusCall();
    return this.nyxIdStatusResult;
  }

  async startNyxidLogin(): Promise<NyxIdView> {
    this.startNyxIdLoginCall();
    const view: NyxIdView = {
      state: "authorizing",
      userCode: "ABCD-1234",
      verificationUrl:
        "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
      expiresAt: "2026-10-09T10:10:00Z",
    };
    this.emitNyxIdView(view);
    return view;
  }

  async cancelNyxidLogin(): Promise<NyxIdView> {
    this.cancelNyxIdLoginCall();
    const view: NyxIdView = { state: "signed_out" };
    this.emitNyxIdView(view);
    return view;
  }

  async refreshNyxidCapabilities(): Promise<NyxIdView> {
    this.refreshNyxIdCall();
    return this.currentNyxId;
  }

  async logoutNyxid(): Promise<NyxIdView> {
    this.logoutNyxIdCall();
    const view: NyxIdView = { state: "signed_out" };
    this.emitNyxIdView(view);
    return view;
  }

  async sendNyxIdChat(
    request: NyxIdChatRequest,
  ): Promise<NyxIdChatCompletedEvent> {
    this.sendNyxIdChatCall(request);
    this.emitNyxIdChatEvent({
      kind: "started",
      requestId: request.requestId,
      conversationId: CHAT_CONVERSATION_ID,
      turnId: "turn-1",
    });
    this.emitNyxIdChatEvent({
      kind: "delta",
      requestId: request.requestId,
      text: "我会通过 NyxID 帮你处理。",
    });
    const completed = {
      kind: "completed",
      requestId: request.requestId,
      conversationId: CHAT_CONVERSATION_ID,
      status: "completed",
      error: null,
    } as const;
    this.emitNyxIdChatEvent(completed);
    return completed;
  }

  async nyxIdChatHistory(conversationId: string): Promise<NyxIdChatHistory> {
    this.nyxIdChatHistoryCall(conversationId);
    return {
      conversation: { id: conversationId, activeTurn: false },
      messages: [],
    };
  }

  async recoverNyxIdChat(): Promise<NyxIdChatRecovery | null> {
    return this.recoverNyxIdChatCall();
  }

  async stopNyxIdChat(conversationId: string): Promise<void> {
    this.stopNyxIdChatCall(conversationId);
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

  async onWindowDrag(
    listener: (event: WindowDragEvent) => void,
  ): Promise<Unlisten> {
    this.windowDragListeners.add(listener);
    return () => {
      this.windowDragListeners.delete(listener);
    };
  }

  async onNyxidChanged(listener: (view: NyxIdView) => void): Promise<Unlisten> {
    this.nyxIdListeners.add(listener);
    return () => {
      this.nyxIdListeners.delete(listener);
    };
  }

  async onNyxIdChatEvent(
    listener: (event: NyxIdChatEvent) => void,
  ): Promise<Unlisten> {
    this.chatListeners.add(listener);
    return () => {
      this.chatListeners.delete(listener);
    };
  }

  dispose(): void {
    this.stateListeners.clear();
    this.mealListeners.clear();
    this.windowDragListeners.clear();
    this.nyxIdListeners.clear();
    this.chatListeners.clear();
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
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    ).toBeInTheDocument();
    expect(runtime.saveSettingsCall).toHaveBeenCalledWith(
      expect.objectContaining({ onboardingComplete: true }),
    );
    await waitFor(() => {
      expect(runtime.setWindowModeCall).toHaveBeenLastCalledWith("compact");
    });
  });

  it("opens NyxID chat from the mascot and streams the assistant reply", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "帮我安排今天的事情");
    await user.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => {
      expect(runtime.sendNyxIdChatCall).toHaveBeenCalledWith(
        expect.objectContaining({ text: "帮我安排今天的事情" }),
      );
    });
    expect(screen.getByText("帮我安排今天的事情")).toBeInTheDocument();
    expect(screen.getByText("我会通过 NyxID 帮你处理。")).toBeInTheDocument();
    expect(composer).toHaveValue("");
  });

  it("routes chat dragging through the runtime without hijacking controls", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );

    const dragHandle = screen.getAllByText("NyxID 已连接")[0]?.parentElement;
    expect(dragHandle).toHaveClass("chat-toolbar-title");
    fireEvent.pointerDown(dragHandle!, {
      button: 0,
      isPrimary: true,
      pointerId: 31,
    });
    expect(runtime.startWindowDragCall).toHaveBeenCalledTimes(1);

    fireEvent.pointerDown(dragHandle!, {
      button: 2,
      isPrimary: true,
      pointerId: 32,
    });
    fireEvent.pointerDown(dragHandle!, {
      button: 0,
      isPrimary: false,
      pointerId: 33,
    });

    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "帮我看看今天的安排");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await user.click(screen.getByRole("button", { name: "收起对话" }));

    expect(runtime.startWindowDragCall).toHaveBeenCalledTimes(1);
  });

  it("replaces an incomplete stream with NyxID's final text snapshot", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementation(async (request) => {
      runtime.emitNyxIdChatEvent({
        kind: "started",
        requestId: request.requestId,
        conversationId: CHAT_CONVERSATION_ID,
        turnId: "turn-snapshot",
      });
      runtime.emitNyxIdChatEvent({
        kind: "delta",
        requestId: request.requestId,
        text: "回复后半段",
      });
      runtime.emitNyxIdChatEvent({
        kind: "snapshot",
        requestId: request.requestId,
        text: "这是完整回复后半段",
      });
      const completed: NyxIdChatCompletedEvent = {
        kind: "completed",
        requestId: request.requestId,
        conversationId: CHAT_CONVERSATION_ID,
        status: "completed",
        error: null,
      };
      runtime.emitNyxIdChatEvent(completed);
      return completed;
    });

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      "给我完整结果",
    );
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("这是完整回复后半段")).toBeInTheDocument();
    expect(screen.queryByText("回复后半段")).not.toBeInTheDocument();
  });

  it("drives the pet movement from native window drag events", async () => {
    const runtime = new TestRuntime(onboardedSnapshot());

    render(<CompanionApp runtime={runtime} />);
    const mascotButton = await screen.findByRole("button", {
      name: /拖动可以移动/,
    });

    act(() => runtime.emitWindowDrag({ phase: "moving", direction: "left" }));
    expect(mascotButton).toHaveAttribute("data-interaction", "dragging-left");
    expect(mascotButton.querySelector(".mascot--running-left")).not.toBeNull();

    act(() => runtime.emitWindowDrag({ phase: "settled", direction: null }));
    expect(mascotButton).toHaveAttribute("data-interaction", "resting");
    expect(mascotButton.querySelector(".mascot--idle")).not.toBeNull();
  });

  it.each([
    {
      status: "failed" as const,
      error: { code: "assistant_failed", message: "稍后再试一次" },
      terminalCopy: "稍后再试一次",
      mascotLabel: "Nyx could not finish the task",
    },
    {
      status: "cancelled" as const,
      error: null,
      terminalCopy: "这次处理已经停止",
      mascotLabel: "Nyx is resting",
    },
  ])(
    "shows partial text and the independent $status terminal state",
    async ({ status, error, terminalCopy, mascotLabel }) => {
      const user = userEvent.setup();
      const runtime = new TestRuntime(
        onboardedSnapshot(),
        Promise.resolve(false),
        Promise.resolve(),
        Promise.resolve(connectedNyxIdView()),
      );
      vi.spyOn(runtime, "sendNyxIdChat").mockImplementation(async (request) => {
        runtime.emitNyxIdChatEvent({
          kind: "started",
          requestId: request.requestId,
          conversationId: CHAT_CONVERSATION_ID,
          turnId: "turn-terminal",
        });
        runtime.emitNyxIdChatEvent({
          kind: "delta",
          requestId: request.requestId,
          text: "已经完成一部分",
        });
        const completed: NyxIdChatCompletedEvent = {
          kind: "completed",
          requestId: request.requestId,
          conversationId: CHAT_CONVERSATION_ID,
          status,
          error,
        };
        runtime.emitNyxIdChatEvent(completed);
        return completed;
      });

      render(<CompanionApp runtime={runtime} />);
      await user.click(
        await screen.findByRole("button", { name: "和 NyxID 对话" }),
      );
      await user.type(
        screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
        "执行一个会返回部分结果的任务",
      );
      await user.click(screen.getByRole("button", { name: "发送" }));

      expect(await screen.findByText("已经完成一部分")).toBeInTheDocument();
      expect(await screen.findByText(terminalCopy)).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "收起对话" }));
      expect(
        screen.getByRole("img", { name: mascotLabel }),
      ).toBeInTheDocument();
    },
  );

  it("keeps sending disabled until the native chat listener is ready", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let finishListener: ((unlisten: Unlisten) => void) | undefined;
    vi.spyOn(runtime, "onNyxIdChatEvent").mockReturnValue(
      new Promise<Unlisten>((resolve) => {
        finishListener = resolve;
      }),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    expect(composer).toBeDisabled();
    expect(screen.getAllByText("正在准备 NyxID 对话")).toHaveLength(2);
    expect(runtime.sendNyxIdChatCall).not.toHaveBeenCalled();

    await act(async () => {
      finishListener?.(() => undefined);
    });
    await waitFor(() => expect(composer).toBeEnabled());
  });

  it("reconciles an interrupted stream from the same NyxID conversation", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let sentRequestId: string | undefined;
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementation(async (request) => {
      sentRequestId = request.requestId;
      runtime.emitNyxIdChatEvent({
        kind: "started",
        requestId: request.requestId,
        conversationId: CHAT_CONVERSATION_ID,
        turnId: "turn-recovery",
      });
      throw new Error("stream disconnected");
    });
    runtime.recoverNyxIdChatCall.mockImplementation(async () => {
      if (!sentRequestId) return null;
      return {
        requestId: sentRequestId,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "message-user",
              seq: 1,
              turnId: "turn-recovery",
              role: "user",
              text: "同意刚才那条申请",
              status: "completed",
              errorCode: null,
            },
            {
              id: "message-assistant",
              seq: 2,
              turnId: "turn-recovery",
              role: "assistant",
              text: "已经通过 NyxID 处理好了。",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      };
    });

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "同意刚才那条申请");
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(
      await screen.findByText("已经通过 NyxID 处理好了。"),
    ).toBeInTheDocument();
    expect(runtime.recoverNyxIdChatCall).toHaveBeenCalled();
    expect(composer).toHaveValue("");
  });

  it("recovers a new-conversation admission unknown through its receipt", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let sentRequest: NyxIdChatRequest | undefined;
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementation(async (request) => {
      sentRequest = request;
      throw new NyxIdChatSendError(
        "admission_unknown",
        "无法确认 NyxID 是否已收到这条消息，请不要重复发送。",
      );
    });
    runtime.recoverNyxIdChatCall.mockImplementation(async () => {
      if (!sentRequest) return null;
      return {
        requestId: sentRequest.requestId,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "new-conversation-user",
              seq: 1,
              turnId: "turn-new-conversation",
              role: "user",
              text: "执行一次且只能执行一次",
              status: "completed",
              errorCode: null,
            },
            {
              id: "new-conversation-result",
              seq: 2,
              turnId: "turn-new-conversation",
              role: "assistant",
              text: "这次操作已完成",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      };
    });

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      ).toBeEnabled(),
    );
    await user.type(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      "执行一次且只能执行一次",
    );
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("这次操作已完成")).toBeInTheDocument();
    expect(sentRequest).not.toHaveProperty("conversationId");
    expect(runtime.recoverNyxIdChatCall).toHaveBeenCalled();
  });

  it("uses the exact receipt before resolving an existing-conversation unknown", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "先建立会话");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByText("我会通过 NyxID 帮你处理。");

    let secondRequest: NyxIdChatRequest | undefined;
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementationOnce(
      async (request) => {
        secondRequest = request;
        throw new NyxIdChatSendError(
          "admission_unknown",
          "无法确认 NyxID 是否已收到这条消息，请不要重复发送。",
        );
      },
    );
    runtime.recoverNyxIdChatCall.mockImplementation(async () => {
      if (!secondRequest) return null;
      return {
        requestId: secondRequest.requestId,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "second-turn-user",
              seq: 3,
              turnId: "turn-second",
              role: "user",
              text: "处理第二个动作",
              status: "completed",
              errorCode: null,
            },
            {
              id: "second-turn-result",
              seq: 4,
              turnId: "turn-second",
              role: "assistant",
              text: "第二个动作已完成",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      };
    });
    await user.type(composer, "处理第二个动作");
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("第二个动作已完成")).toBeInTheDocument();
    expect(secondRequest).toMatchObject({
      conversationId: CHAT_CONVERSATION_ID,
      text: "处理第二个动作",
    });
    expect(runtime.recoverNyxIdChatCall).toHaveBeenCalled();
  });

  it("restores an active NyxID turn after restart without reposting its message", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let finishRecovery:
      ((recovery: NyxIdChatRecovery | null) => void) | undefined;
    runtime.recoverNyxIdChatCall
      .mockResolvedValueOnce({
        requestId: CHAT_RECOVERY_REQUEST_ID,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: true },
          messages: [
            {
              id: "restarted-user-message",
              seq: 1,
              turnId: "turn-restarted",
              role: "user",
              text: "同意这条请假申请",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      })
      .mockImplementationOnce(
        () =>
          new Promise<NyxIdChatRecovery | null>((resolve) => {
            finishRecovery = resolve;
          }),
      );

    render(<CompanionApp runtime={runtime} />);
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledTimes(2),
    );
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );

    expect(await screen.findByText("同意这条请假申请")).toBeInTheDocument();
    expect(screen.getByText("正在确认处理结果")).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
    ).toBeDisabled();
    expect(runtime.sendNyxIdChatCall).not.toHaveBeenCalled();

    await act(async () => {
      finishRecovery?.({
        requestId: CHAT_RECOVERY_REQUEST_ID,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "restarted-user-message",
              seq: 1,
              turnId: "turn-restarted",
              role: "user",
              text: "同意这条请假申请",
              status: "completed",
              errorCode: null,
            },
            {
              id: "restarted-assistant-message",
              seq: 2,
              turnId: "turn-restarted",
              role: "assistant",
              text: "已经通过 NyxID 处理好了。",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      });
    });
    expect(
      await screen.findByText("已经通过 NyxID 处理好了。"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
    ).toBeEnabled();
  });

  it("recovers each owner's pending turn when switching away and back", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    runtime.recoverNyxIdChatCall
      .mockResolvedValueOnce({
        requestId: CHAT_RECOVERY_REQUEST_ID,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: true },
          messages: [
            {
              id: "owner-one-user-message",
              seq: 1,
              turnId: "turn-owner-one",
              role: "user",
              text: "第一位用户未完成的任务",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      })
      .mockImplementationOnce(
        () => new Promise<NyxIdChatRecovery | null>(() => undefined),
      )
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        requestId: CHAT_RECOVERY_REQUEST_ID,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "owner-one-result",
              seq: 2,
              turnId: "turn-owner-one",
              role: "assistant",
              text: "第一位用户的任务已经完成",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      });
    render(<CompanionApp runtime={runtime} />);
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledTimes(2),
    );
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    expect(
      await screen.findByText("第一位用户未完成的任务"),
    ).toBeInTheDocument();

    act(() => {
      runtime.emitNyxIdView({
        ...connectedNyxIdView(),
        user: {
          id: "user-2",
          email: "second@example.com",
          displayName: "第二位用户",
        },
      });
    });
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledTimes(3),
    );
    expect(
      screen.queryByText("第一位用户未完成的任务"),
    ).not.toBeInTheDocument();

    act(() => runtime.emitNyxIdView(connectedNyxIdView()));
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledTimes(4),
    );
    expect(
      await screen.findByText("第一位用户的任务已经完成"),
    ).toBeInTheDocument();
    expect(runtime.sendNyxIdChatCall).not.toHaveBeenCalled();
  });

  it("stops the active NyxID turn from the composer", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let sentRequestId: string | undefined;
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementation((request) => {
      sentRequestId = request.requestId;
      runtime.emitNyxIdChatEvent({
        kind: "started",
        requestId: request.requestId,
        conversationId: CHAT_CONVERSATION_ID,
        turnId: "turn-running",
      });
      return new Promise<NyxIdChatCompletedEvent>(() => undefined);
    });
    runtime.recoverNyxIdChatCall.mockImplementation(async () => {
      if (!sentRequestId) return null;
      return {
        requestId: sentRequestId,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "message-user",
              seq: 1,
              turnId: "turn-running",
              role: "user",
              text: "执行一个长任务",
              status: "completed",
              errorCode: null,
            },
            {
              id: "message-assistant",
              seq: 2,
              turnId: "turn-running",
              role: "assistant",
              text: "已经完成一部分",
              status: "failed",
              errorCode: "cancelled",
            },
          ],
        },
      };
    });

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      "执行一个长任务",
    );
    await user.click(screen.getByRole("button", { name: "发送" }));
    await user.click(await screen.findByRole("button", { name: "停止处理" }));

    await waitFor(() => {
      expect(runtime.stopNyxIdChatCall).toHaveBeenCalledWith(
        CHAT_CONVERSATION_ID,
      );
    });
    expect(await screen.findByText("已经完成一部分")).toBeInTheDocument();
    expect(await screen.findByText("这次处理已经停止")).toBeInTheDocument();
    expect(runtime.recoverNyxIdChatCall).toHaveBeenCalled();
  });

  it("keeps an admission with no conversation id fenced as unknown", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    vi.spyOn(runtime, "sendNyxIdChat").mockRejectedValue(
      new Error("连接中断。"),
    );

    render(<CompanionApp runtime={runtime} />);
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledOnce(),
    );
    runtime.recoverNyxIdChatCall.mockImplementation(
      () => new Promise<NyxIdChatRecovery | null>(() => undefined),
    );
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "执行一次操作");
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(
      await screen.findByText(/执行状态无法确认，请不要重复发送/),
    ).toBeInTheDocument();
    expect(composer).toHaveValue("");
    expect(composer).toBeDisabled();
    expect(screen.getByRole("button", { name: "停止处理" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "开始新对话" })).toBeDisabled();
  });

  it("releases the local turn after a definite send rejection", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    vi.spyOn(runtime, "sendNyxIdChat").mockRejectedValue(
      new NyxIdChatSendError("rejected", "NyxID 余额不足，这次对话没有执行。"),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "执行一次操作");
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(
      await screen.findByText("NyxID 余额不足，这次对话没有执行。"),
    ).toBeInTheDocument();
    expect(composer).toBeEnabled();
    expect(screen.getByRole("button", { name: "开始新对话" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("preserves an unknown active turn across transient account states", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    vi.spyOn(runtime, "sendNyxIdChat").mockRejectedValue(
      new Error("stream disconnected"),
    );

    render(<CompanionApp runtime={runtime} />);
    await waitFor(() =>
      expect(runtime.recoverNyxIdChatCall).toHaveBeenCalledOnce(),
    );
    runtime.recoverNyxIdChatCall.mockImplementation(
      () => new Promise<NyxIdChatRecovery | null>(() => undefined),
    );
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "只能执行一次的操作");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(
      await screen.findByText(/执行状态无法确认，请不要重复发送/),
    ).toBeInTheDocument();

    act(() => runtime.emitNyxIdView({ state: "checking" }));
    expect(screen.getByText("只能执行一次的操作")).toBeInTheDocument();
    act(() =>
      runtime.emitNyxIdView({
        state: "error",
        error: {
          code: "temporary_failure",
          message: "暂时无法刷新账号状态",
          retryable: true,
          retryAction: "refresh",
        },
      }),
    );
    expect(screen.getByText("只能执行一次的操作")).toBeInTheDocument();
    act(() => runtime.emitNyxIdView(connectedNyxIdView()));

    expect(screen.getByText("只能执行一次的操作")).toBeInTheDocument();
    expect(composer).toBeDisabled();
    expect(screen.getByRole("button", { name: "开始新对话" })).toBeDisabled();
  });

  it("clears an unsent draft when the connected account changes", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      "第一位用户尚未发送的草稿",
    );

    act(() => {
      runtime.emitNyxIdView({
        ...connectedNyxIdView(),
        user: {
          id: "user-2",
          email: "second@example.com",
          displayName: "第二位用户",
        },
      });
    });

    expect(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
    ).toHaveValue("");
  });

  it("clears transcript authority when the connected NyxID user changes", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    const composer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await user.type(composer, "第一位用户的消息");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(await screen.findByText("第一位用户的消息")).toBeInTheDocument();

    act(() => {
      runtime.emitNyxIdView({
        ...connectedNyxIdView(),
        user: {
          id: "user-2",
          email: "second@example.com",
          displayName: "第二位用户",
        },
      });
    });
    expect(screen.queryByText("第一位用户的消息")).not.toBeInTheDocument();

    runtime.sendNyxIdChatCall.mockClear();
    const secondComposer = screen.getByRole("textbox", {
      name: "给 NyxID 发送消息",
    });
    await waitFor(() => expect(secondComposer).toBeEnabled());
    await user.type(secondComposer, "第二位用户的消息");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(runtime.sendNyxIdChatCall).toHaveBeenCalled());
    expect(runtime.sendNyxIdChatCall.mock.calls[0]?.[0]).not.toHaveProperty(
      "conversationId",
    );
  });

  it("ignores an old admission recovery response after the NyxID account changes", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      Promise.resolve(connectedNyxIdView()),
    );
    let sentRequestId: string | undefined;
    let finishRecovery:
      ((recovery: NyxIdChatRecovery | null) => void) | undefined;
    let createdOldRecovery = false;
    vi.spyOn(runtime, "sendNyxIdChat").mockImplementation(async (request) => {
      sentRequestId = request.requestId;
      runtime.emitNyxIdChatEvent({
        kind: "started",
        requestId: request.requestId,
        conversationId: CHAT_CONVERSATION_ID,
        turnId: "turn-old-account",
      });
      throw new Error("stream disconnected");
    });
    runtime.recoverNyxIdChatCall.mockImplementation(() => {
      if (!sentRequestId) return Promise.resolve(null);
      if (createdOldRecovery) return Promise.resolve(null);
      createdOldRecovery = true;
      return new Promise<NyxIdChatRecovery | null>((resolve) => {
        finishRecovery = resolve;
      });
    });

    render(<CompanionApp runtime={runtime} />);
    await user.click(
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
      "旧账号操作",
    );
    await user.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(finishRecovery).toBeTypeOf("function"));

    act(() => {
      runtime.emitNyxIdView({
        ...connectedNyxIdView(),
        user: {
          id: "user-2",
          email: "second@example.com",
          displayName: "第二位用户",
        },
      });
    });
    await act(async () => {
      finishRecovery?.({
        requestId: sentRequestId!,
        history: {
          conversation: { id: CHAT_CONVERSATION_ID, activeTurn: false },
          messages: [
            {
              id: "old-answer",
              seq: 1,
              turnId: "turn-old-account",
              role: "assistant",
              text: "旧账号的完成结果",
              status: "completed",
              errorCode: null,
            },
          ],
        },
      });
    });

    expect(screen.queryByText("旧账号的完成结果")).not.toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "给 NyxID 发送消息" }),
    ).toBeEnabled();
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

  it("starts NyxID device login from settings and can open Assistant once connected", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());
    const openSpy = vi.spyOn(runtime, "openNyxidAssistant");

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));

    expect(
      screen.getByRole("heading", { name: "陪伴设置" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "连接" }));
    await waitFor(() => {
      expect(runtime.startNyxIdLoginCall).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByLabelText("NyxID 登录确认码")).toHaveTextContent(
      "ABCD-1234",
    );

    act(() => runtime.emitNyxIdView(connectedNyxIdView()));
    expect(screen.getByText("xiaokui@example.com")).toBeInTheDocument();
    expect(screen.getByText("GitHub")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "打开 NyxID Assistant" }),
    );
    await waitFor(() => expect(openSpy).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: "现在试一次提醒" }));
    expect(
      await screen.findByRole("heading", { name: "小葵，今天想怎么吃？" }),
    ).toBeInTheDocument();
    expect(runtime.triggerDemoCall).toHaveBeenCalledTimes(1);
  });

  it("cancels a pending NyxID login and retries terminal outcomes", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "连接" }));
    expect(await screen.findByText("等待浏览器确认")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "取消连接" }));
    await waitFor(() => {
      expect(runtime.cancelNyxIdLoginCall).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByText("未连接")).toBeInTheDocument();

    act(() =>
      runtime.emitNyxIdView({
        state: "denied",
        message: "这次连接没有被批准",
      }),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("这次连接没有被批准");
    await user.click(screen.getByRole("button", { name: "重新连接" }));
    expect(runtime.startNyxIdLoginCall).toHaveBeenCalledTimes(2);

    act(() =>
      runtime.emitNyxIdView({
        state: "expired",
        message: "确认已过期，请重新连接",
      }),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("确认已过期");
  });

  it.each([
    {
      retryAction: "connect",
      label: "重试连接",
      calls: (runtime: TestRuntime) => runtime.startNyxIdLoginCall,
    },
    {
      retryAction: "cancel",
      label: "重试取消",
      calls: (runtime: TestRuntime) => runtime.cancelNyxIdLoginCall,
    },
    {
      retryAction: "refresh",
      label: "重试刷新",
      calls: (runtime: TestRuntime) => runtime.refreshNyxIdCall,
    },
    {
      retryAction: "logout",
      label: "重试断开",
      calls: (runtime: TestRuntime) => runtime.logoutNyxIdCall,
    },
  ] as const)(
    "retries the originating NyxID $retryAction action",
    async ({ retryAction, label, calls }) => {
      const user = userEvent.setup();
      const runtime = new TestRuntime(onboardedSnapshot());

      render(<CompanionApp runtime={runtime} />);
      await user.click(await screen.findByRole("button", { name: "设置" }));
      act(() =>
        runtime.emitNyxIdView({
          state: "error",
          error: {
            code: `${retryAction}_failed`,
            message: "刚才没有完成",
            retryable: true,
            retryAction,
          },
        }),
      );

      await user.click(screen.getByRole("button", { name: label }));
      await waitFor(() => expect(calls(runtime)).toHaveBeenCalledTimes(1));
    },
  );

  it("does not offer a retry for a terminal NyxID error", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    act(() =>
      runtime.emitNyxIdView({
        state: "error",
        error: {
          code: "auth_device_already_delivered",
          message: "这次登录已经被领取",
          retryable: false,
          retryAction: null,
        },
      }),
    );

    expect(screen.getByRole("alert")).toHaveTextContent("这次登录已经被领取");
    expect(
      screen.queryByRole("button", { name: /重试|重新连接/ }),
    ).not.toBeInTheDocument();
  });

  it("refreshes the connected service summary and disconnects locally", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    act(() => runtime.emitNyxIdView(connectedNyxIdView()));

    expect(screen.getByLabelText("NyxID 服务状态")).toHaveTextContent(
      "2已启用1需处理1已停用",
    );
    await user.click(
      screen.getByRole("button", { name: "刷新 NyxID 服务状态" }),
    );
    await waitFor(() => {
      expect(runtime.refreshNyxIdCall).toHaveBeenCalledTimes(1);
    });

    await user.click(screen.getByRole("button", { name: "断开 NyxID" }));
    await waitFor(() => {
      expect(runtime.logoutNyxIdCall).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByText("未连接")).toBeInTheDocument();
  });

  it("keeps meal settings editable while a NyxID action is pending", async () => {
    const user = userEvent.setup();
    const runtime = new TestRuntime(onboardedSnapshot());
    let finishLogin: ((view: NyxIdView) => void) | undefined;
    const startLogin = vi.spyOn(runtime, "startNyxidLogin").mockImplementation(
      () =>
        new Promise<NyxIdView>((resolve) => {
          finishLogin = resolve;
        }),
    );

    render(<CompanionApp runtime={runtime} />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "连接" }));
    await waitFor(() => expect(startLogin).toHaveBeenCalled());

    const companionName = screen.getByRole("textbox", { name: "搭子名字" });
    expect(companionName).toBeEnabled();
    await user.clear(companionName);
    await user.type(companionName, "Momo");
    expect(companionName).toHaveValue("Momo");

    await act(async () => {
      finishLogin?.({
        state: "authorizing",
        userCode: "ABCD-1234",
        verificationUrl:
          "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
        expiresAt: "2026-10-09T10:10:00Z",
      });
    });
  });

  it("does not overwrite a live NyxID event with stale boot status", async () => {
    const user = userEvent.setup();
    let finishStatus: ((view: NyxIdView) => void) | undefined;
    const statusResult = new Promise<NyxIdView>((resolve) => {
      finishStatus = resolve;
    });
    const runtime = new TestRuntime(
      onboardedSnapshot(),
      Promise.resolve(false),
      Promise.resolve(),
      statusResult,
    );

    render(<CompanionApp runtime={runtime} />);
    await waitFor(() => expect(runtime.nyxIdStatusCall).toHaveBeenCalled());
    await act(async () => {
      runtime.emitNyxIdView(connectedNyxIdView());
      finishStatus?.({ state: "signed_out" });
    });

    await user.click(await screen.findByRole("button", { name: "设置" }));
    expect(screen.getByText("xiaokui@example.com")).toBeInTheDocument();
    expect(screen.queryByText("未连接")).not.toBeInTheDocument();
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
      await screen.findByRole("button", { name: "和 NyxID 对话" }),
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
