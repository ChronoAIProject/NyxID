import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent,
} from "react";

import { CelebrationPanel } from "./components/celebration-panel";
import { ChatPanel, type CompanionChatMessage } from "./components/chat-panel";
import { IdleView } from "./components/idle-view";
import type { MascotState } from "./components/mascot";
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
  NyxIdChatSendError,
  nyxIdChatRecoveryDelayMs,
  type CompanionRuntime,
  type NyxIdChatEvent,
  type NyxIdChatHistory,
  type NyxIdChatRecovery,
  type NyxIdView,
  type Unlisten,
  type WindowDragEvent,
} from "./runtime";

type Screen = "idle" | "chat" | "meal" | "settings" | "celebration";

interface Celebration {
  readonly choice: string;
}

interface CompanionAppProps {
  readonly runtime?: CompanionRuntime;
}

type ChatListenerState =
  | { readonly status: "connecting" | "ready" }
  | { readonly status: "failed"; readonly message: string };

interface ActiveChatTurn {
  readonly requestId: string;
  readonly generation: number;
  conversationId?: string;
  recovery?: Promise<void>;
}

let sharedRuntime: CompanionRuntime | undefined;

const WINDOW_DRAG_HANDLE_SELECTOR = "[data-window-drag-handle]";
const WINDOW_DRAG_CONTROL_SELECTOR = [
  "button",
  "a[href]",
  "input",
  "label",
  "textarea",
  "select",
  "[contenteditable]:not([contenteditable='false'])",
  "[role='button']",
  "[role='checkbox']",
  "[role='link']",
  "[role='menuitem']",
  "[role='radio']",
  "[role='slider']",
  "[role='spinbutton']",
  "[role='switch']",
  "[role='tab']",
  "[role='textbox']",
  "[data-window-drag-exclude]",
].join(",");

function isWindowDragPointer(event: PointerEvent<HTMLDivElement>): boolean {
  if (event.button !== 0 || !event.isPrimary) return false;
  if (!(event.target instanceof Element)) return false;

  const handle = event.target.closest(WINDOW_DRAG_HANDLE_SELECTOR);
  if (!handle || !event.currentTarget.contains(handle)) return false;
  return !event.target.closest(WINDOW_DRAG_CONTROL_SELECTOR);
}

function defaultRuntime(): CompanionRuntime {
  sharedRuntime ??= createCompanionRuntime();
  return sharedRuntime;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "发生了意外，请稍后再试";
}

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}

function messagesFromHistory(
  history: NyxIdChatHistory,
): CompanionChatMessage[] {
  return history.messages
    .filter(
      (message): message is typeof message & { role: "user" | "assistant" } =>
        message.role === "user" || message.role === "assistant",
    )
    .map((message) => {
      const cancelledAssistant =
        message.role === "assistant" && message.errorCode === "cancelled";
      const failedAssistant =
        message.role === "assistant" &&
        message.status === "failed" &&
        !cancelledAssistant;
      return {
        id: message.id,
        role: message.role,
        text:
          message.text ||
          (cancelledAssistant || failedAssistant
            ? ""
            : message.status === "failed"
              ? "这次处理没有完成"
              : "处理完成"),
        ...(cancelledAssistant
          ? {
              terminalStatus: "cancelled" as const,
              terminalMessage: "这次处理已经停止",
            }
          : failedAssistant
            ? {
                terminalStatus: "failed" as const,
                terminalMessage: "这次处理没有完成",
              }
            : {}),
      };
    });
}

function messagesFromActiveHistory(
  recovery: NyxIdChatRecovery,
): CompanionChatMessage[] {
  const messages = messagesFromHistory(recovery.history);
  const last = messages.at(-1);
  if (last?.role === "assistant" && !last.terminalStatus) {
    return messages.map((message, index) =>
      index === messages.length - 1 ? { ...message, pending: true } : message,
    );
  }
  return [
    ...messages,
    {
      id: recovery.requestId,
      role: "assistant",
      text: "",
      pending: true,
    },
  ];
}

function historyOutcome(history: NyxIdChatHistory): MascotState {
  const lastMessage = history.messages.at(-1);
  if (lastMessage?.errorCode === "cancelled") return "idle";
  return lastMessage?.status === "failed" ? "failed" : "review";
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
  const [chatMessages, setChatMessages] = useState<CompanionChatMessage[]>([]);
  const [chatConversationId, setChatConversationId] = useState<string>();
  const [chatPending, setChatPending] = useState(false);
  const [chatStopping, setChatStopping] = useState(false);
  const [chatRecovering, setChatRecovering] = useState(false);
  const [chatOutcome, setChatOutcome] = useState<MascotState>();
  const [chatListener, setChatListener] = useState<ChatListenerState>({
    status: "connecting",
  });
  const [chatListenerRevision, setChatListenerRevision] = useState(0);
  const [chatOwnerKey, setChatOwnerKey] = useState("signed-out");
  const [windowDragEvent, setWindowDragEvent] = useState<WindowDragEvent>();
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
  const chatGeneration = useRef(0);
  const activeChatTurn = useRef<ActiveChatTurn>();
  const chatConversationIdRef = useRef<string>();
  const chatOwnerId = useRef<string>();

  const resetChat = useCallback(() => {
    chatGeneration.current += 1;
    activeChatTurn.current = undefined;
    chatConversationIdRef.current = undefined;
    setChatConversationId(undefined);
    setChatMessages([]);
    setChatPending(false);
    setChatStopping(false);
    setChatRecovering(false);
    setChatOutcome(undefined);
  }, []);

  const applyNyxIdView = useCallback(
    (next: NyxIdView) => {
      if (next.state === "connected") {
        if (chatOwnerId.current !== next.user.id) {
          chatOwnerId.current = next.user.id;
          setChatOwnerKey(next.user.id);
          resetChat();
        }
      } else if (next.state !== "checking" && next.state !== "error") {
        if (chatOwnerId.current !== undefined) {
          chatOwnerId.current = undefined;
          setChatOwnerKey("signed-out");
          resetChat();
        }
      }
      setNyxIdView(next);
    },
    [resetChat],
  );

  const ownsChatTurn = useCallback((turn: ActiveChatTurn): boolean => {
    return (
      activeChatTurn.current === turn &&
      chatGeneration.current === turn.generation
    );
  }, []);

  const finishChatFromHistory = useCallback(
    (turn: ActiveChatTurn, history: NyxIdChatHistory): void => {
      if (history.conversation.activeTurn || !ownsChatTurn(turn)) return;
      chatConversationIdRef.current = history.conversation.id;
      setChatConversationId(history.conversation.id);
      setChatMessages(messagesFromHistory(history));
      setChatPending(false);
      setChatStopping(false);
      setChatRecovering(false);
      setChatOutcome(historyOutcome(history));
      activeChatTurn.current = undefined;
    },
    [ownsChatTurn],
  );

  const finishChatFromEvent = useCallback(
    (
      turn: ActiveChatTurn,
      event: Extract<NyxIdChatEvent, { kind: "completed" }>,
    ): void => {
      if (!ownsChatTurn(turn)) return;
      chatConversationIdRef.current = event.conversationId;
      setChatConversationId(event.conversationId);
      setChatPending(false);
      setChatStopping(false);
      setChatRecovering(false);
      setChatOutcome(
        event.status === "failed"
          ? "failed"
          : event.status === "cancelled"
            ? "idle"
            : "review",
      );
      setChatMessages((current) =>
        current.map((message) => {
          if (message.id !== event.requestId || message.role !== "assistant") {
            return message;
          }
          if (event.status === "failed") {
            return {
              ...message,
              pending: false,
              terminalStatus: "failed" as const,
              terminalMessage: event.error?.message || "这次处理没有完成",
            };
          }
          if (event.status === "cancelled") {
            return {
              ...message,
              pending: false,
              terminalStatus: "cancelled" as const,
              terminalMessage: "这次处理已经停止",
            };
          }
          return {
            ...message,
            pending: false,
            text: message.text || "处理完成",
          };
        }),
      );
      activeChatTurn.current = undefined;
    },
    [ownsChatTurn],
  );

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
    let unlisten: Unlisten | undefined;
    setChatListener({ status: "connecting" });

    function applyChatEvent(event: NyxIdChatEvent) {
      const turn = activeChatTurn.current;
      if (!alive || !turn || event.requestId !== turn.requestId) return;
      if (event.kind === "started") {
        turn.conversationId = event.conversationId;
        chatConversationIdRef.current = event.conversationId;
        setChatConversationId(event.conversationId);
        return;
      }
      if (event.kind === "delta") {
        setChatMessages((current) =>
          current.map((message) =>
            message.id === event.requestId && message.role === "assistant"
              ? { ...message, text: message.text + event.text }
              : message,
          ),
        );
        return;
      }
      if (event.kind === "snapshot") {
        setChatMessages((current) =>
          current.map((message) =>
            message.id === event.requestId && message.role === "assistant"
              ? { ...message, text: event.text }
              : message,
          ),
        );
        return;
      }
      finishChatFromEvent(turn, event);
    }

    async function subscribe() {
      try {
        unlisten = await runtime.onNyxIdChatEvent(applyChatEvent);
        if (!alive) {
          await unlisten();
          return;
        }
        setChatListener({ status: "ready" });
      } catch (error) {
        if (alive) {
          setChatListener({
            status: "failed",
            message: errorMessage(error),
          });
        }
      }
    }

    void subscribe();
    return () => {
      alive = false;
      if (unlisten) void unlisten();
    };
  }, [chatListenerRevision, finishChatFromEvent, runtime]);

  useEffect(() => {
    let alive = true;
    let receivedLiveState = false;
    let unlisten: Unlisten | undefined;

    async function bootNyxId() {
      try {
        unlisten = await runtime.onNyxidChanged((next) => {
          if (!alive) return;
          receivedLiveState = true;
          applyNyxIdView(next);
        });
        if (!alive) {
          await unlisten();
          return;
        }

        const initial = await runtime.nyxidStatus();
        if (alive && !receivedLiveState) {
          applyNyxIdView(initial);
        }
      } catch (error) {
        if (alive) {
          applyNyxIdView({
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
  }, [applyNyxIdView, runtime]);

  useEffect(() => {
    let alive = true;
    let unlisten: Unlisten | undefined;

    void runtime
      .onWindowDrag((event) => {
        if (alive) setWindowDragEvent(event);
      })
      .then((cleanup) => {
        if (!alive) {
          void cleanup();
          return;
        }
        unlisten = cleanup;
      })
      .catch(() => undefined);

    return () => {
      alive = false;
      if (unlisten) void unlisten();
    };
  }, [runtime]);

  useEffect(
    () => () => {
      chatGeneration.current += 1;
      activeChatTurn.current = undefined;
    },
    [],
  );

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

  const finishChatNotAdmitted = useCallback(
    (turn: ActiveChatTurn): void => {
      if (!ownsChatTurn(turn)) return;
      setChatMessages((current) =>
        current.map((message) =>
          message.id === turn.requestId && message.role === "assistant"
            ? {
                ...message,
                pending: false,
                terminalStatus: "failed" as const,
                terminalMessage: "NyxID 没有受理这条消息，请重新发送。",
              }
            : message,
        ),
      );
      setChatPending(false);
      setChatStopping(false);
      setChatRecovering(false);
      setChatOutcome("failed");
      activeChatTurn.current = undefined;
    },
    [ownsChatTurn],
  );

  const recoverAdmission = useCallback(
    async (turn: ActiveChatTurn): Promise<void> => {
      setChatRecovering(true);
      let recoveryAttempt = 0;

      while (ownsChatTurn(turn)) {
        try {
          const recovery = await runtime.recoverNyxIdChat();
          if (!ownsChatTurn(turn)) return;
          if (!recovery) {
            finishChatNotAdmitted(turn);
            return;
          }
          if (recovery.requestId !== turn.requestId) {
            throw new Error("NyxID 返回了其他消息的受理记录");
          }
          turn.conversationId = recovery.history.conversation.id;
          chatConversationIdRef.current = recovery.history.conversation.id;
          setChatConversationId(recovery.history.conversation.id);
          if (!recovery.history.conversation.activeTurn) {
            finishChatFromHistory(turn, recovery.history);
            return;
          }
          setChatMessages(messagesFromActiveHistory(recovery));
          setChatPending(true);
          setChatRecovering(true);
        } catch {
          if (!ownsChatTurn(turn)) return;
        }
        await wait(nyxIdChatRecoveryDelayMs(recoveryAttempt));
        recoveryAttempt += 1;
      }
    },
    [finishChatFromHistory, finishChatNotAdmitted, ownsChatTurn, runtime],
  );

  const ensureChatRecovery = useCallback(
    (turn: ActiveChatTurn): Promise<void> => {
      if (turn.recovery) return turn.recovery;
      const recovery = recoverAdmission(turn);
      turn.recovery = recovery;
      void recovery.finally(() => {
        if (turn.recovery === recovery) turn.recovery = undefined;
      });
      return recovery;
    },
    [recoverAdmission],
  );

  const connectedChatOwnerId =
    nyxIdView.state === "connected" ? nyxIdView.user.id : undefined;

  useEffect(() => {
    if (
      chatListener.status !== "ready" ||
      !connectedChatOwnerId ||
      activeChatTurn.current
    ) {
      return;
    }

    let alive = true;
    const ownerUserId = connectedChatOwnerId;
    const generation = chatGeneration.current + 1;
    chatGeneration.current = generation;
    setChatRecovering(true);

    function stillCurrent(): boolean {
      return (
        alive &&
        chatGeneration.current === generation &&
        chatOwnerId.current === ownerUserId
      );
    }

    async function restorePendingChat() {
      let recoveryAttempt = 0;
      while (stillCurrent() && !activeChatTurn.current) {
        try {
          const recovery = await runtime.recoverNyxIdChat();
          if (!stillCurrent() || activeChatTurn.current) return;
          if (!recovery) {
            setChatRecovering(false);
            return;
          }

          const turn: ActiveChatTurn = {
            requestId: recovery.requestId,
            generation,
            conversationId: recovery.history.conversation.id,
          };
          activeChatTurn.current = turn;
          chatConversationIdRef.current = recovery.history.conversation.id;
          setChatConversationId(recovery.history.conversation.id);
          setChatStopping(false);
          setChatOutcome(undefined);
          if (recovery.history.conversation.activeTurn) {
            setChatMessages(messagesFromActiveHistory(recovery));
            setChatPending(true);
            setChatRecovering(true);
            void ensureChatRecovery(turn);
          } else {
            finishChatFromHistory(turn, recovery.history);
          }
          return;
        } catch {
          if (!stillCurrent() || activeChatTurn.current) return;
        }
        await wait(nyxIdChatRecoveryDelayMs(recoveryAttempt));
        recoveryAttempt += 1;
      }
    }

    void restorePendingChat();
    return () => {
      alive = false;
    };
  }, [
    chatListener.status,
    chatOwnerKey,
    connectedChatOwnerId,
    ensureChatRecovery,
    finishChatFromHistory,
    runtime,
  ]);

  async function sendChat(text: string): Promise<void> {
    if (
      activeChatTurn.current ||
      chatPending ||
      chatRecovering ||
      chatListener.status !== "ready" ||
      nyxIdView.state !== "connected"
    ) {
      return;
    }
    const requestId = crypto.randomUUID();
    const generation = chatGeneration.current + 1;
    chatGeneration.current = generation;
    const turn: ActiveChatTurn = {
      requestId,
      generation,
      ...(chatConversationIdRef.current
        ? { conversationId: chatConversationIdRef.current }
        : {}),
    };
    activeChatTurn.current = turn;
    setChatPending(true);
    setChatStopping(false);
    setChatRecovering(false);
    setChatOutcome(undefined);
    setChatMessages((current) => [
      ...current,
      { id: `${requestId}:user`, role: "user", text },
      { id: requestId, role: "assistant", text: "", pending: true },
    ]);
    try {
      const completed = await runtime.sendNyxIdChat({
        requestId,
        ...(turn.conversationId ? { conversationId: turn.conversationId } : {}),
        text,
      });
      if (!ownsChatTurn(turn)) return;
      finishChatFromEvent(turn, completed);
    } catch (error) {
      if (!ownsChatTurn(turn)) return;
      if (error instanceof NyxIdChatSendError && error.kind === "rejected") {
        setChatMessages((current) =>
          current.map((entry) =>
            entry.id === requestId
              ? {
                  ...entry,
                  pending: false,
                  terminalStatus: "failed" as const,
                  terminalMessage: error.message,
                }
              : entry,
          ),
        );
        setChatPending(false);
        setChatStopping(false);
        setChatRecovering(false);
        setChatOutcome("failed");
        activeChatTurn.current = undefined;
        return;
      }
      setChatRecovering(true);
      setChatMessages((current) =>
        current.map((entry) =>
          entry.id === requestId
            ? {
                ...entry,
                pending: true,
                text:
                  entry.text ||
                  (error instanceof NyxIdChatSendError
                    ? error.message
                    : `${errorMessage(error)} 执行状态无法确认，请不要重复发送。`),
              }
            : entry,
        ),
      );
      await ensureChatRecovery(turn);
    }
  }

  async function stopChat(): Promise<void> {
    const turn = activeChatTurn.current;
    const conversationId = turn?.conversationId;
    if (!chatPending || !turn || !conversationId || chatStopping) return;
    setChatStopping(true);
    try {
      await runtime.stopNyxIdChat(conversationId);
      if (!ownsChatTurn(turn)) return;
      await ensureChatRecovery(turn);
    } catch (error) {
      if (ownsChatTurn(turn)) {
        setStatus(`停止失败：${errorMessage(error)}`);
      }
    } finally {
      if (ownsChatTurn(turn)) setChatStopping(false);
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
  } else if (screen === "chat") {
    content = (
      <ChatPanel
        key={chatOwnerKey}
        companionName={snapshot.settings.companionName}
        messages={chatMessages}
        nyxIdView={nyxIdView}
        pending={chatPending}
        stopping={chatStopping}
        recovering={chatRecovering}
        listenerState={chatListener}
        canStop={chatPending && Boolean(chatConversationId)}
        onBack={() => setScreen("idle")}
        onConnect={async () => {
          applyNyxIdView(await runtime.startNyxidLogin());
        }}
        onNewConversation={() => {
          if (!activeChatTurn.current) resetChat();
        }}
        onOpenSettings={() => setScreen("settings")}
        onRetryListener={() => {
          setChatListenerRevision((current) => current + 1);
        }}
        onSend={sendChat}
        onStop={stopChat}
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
          applyNyxIdView(await runtime.startNyxidLogin());
        }}
        onCancelNyxId={async () => {
          applyNyxIdView(await runtime.cancelNyxidLogin());
        }}
        onRefreshNyxId={async () => {
          applyNyxIdView(await runtime.refreshNyxidCapabilities());
        }}
        onLogoutNyxId={async () => {
          applyNyxIdView(await runtime.logoutNyxid());
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
        mascotState={chatPending ? "running" : chatOutcome}
        dragEvent={windowDragEvent}
        onOpen={() => {
          setChatOutcome(undefined);
          setScreen("chat");
        }}
        onMeal={() => {
          if (activePrompt) setScreen("meal");
          else void triggerMealPicker();
        }}
        onSettings={() => setScreen("settings")}
        onStartDrag={() => runtime.startWindowDrag()}
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
      onPointerDown={(event) => {
        if (!isWindowDragPointer(event)) return;
        void runtime.startWindowDrag().catch(() => undefined);
      }}
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
