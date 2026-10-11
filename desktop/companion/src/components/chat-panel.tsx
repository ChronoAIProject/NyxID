import {
  ArrowLeft,
  Cloud,
  LoaderCircle,
  MessageCircle,
  RefreshCw,
  Send,
  Settings,
  Square,
  SquarePen,
} from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import type { NyxIdView } from "../runtime";
import { Mascot, type MascotState } from "./mascot";

export interface CompanionChatMessage {
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly text: string;
  readonly pending?: boolean;
  readonly terminalStatus?: "failed" | "cancelled";
  readonly terminalMessage?: string;
}

interface ChatPanelProps {
  readonly companionName: string;
  readonly messages: readonly CompanionChatMessage[];
  readonly nyxIdView: NyxIdView;
  readonly pending: boolean;
  readonly stopping: boolean;
  readonly recovering: boolean;
  readonly listenerState:
    | { readonly status: "connecting" | "ready" }
    | { readonly status: "failed"; readonly message: string };
  readonly canStop: boolean;
  readonly onBack: () => void;
  readonly onConnect: () => Promise<void>;
  readonly onNewConversation: () => void;
  readonly onOpenSettings: () => void;
  readonly onRetryListener: () => void;
  readonly onSend: (text: string) => Promise<void>;
  readonly onStop: () => Promise<void>;
}

function connectionCopy(
  view: NyxIdView,
  listenerState: ChatPanelProps["listenerState"],
): string {
  switch (view.state) {
    case "connected":
      if (listenerState.status === "connecting") return "正在准备 NyxID 对话";
      if (listenerState.status === "failed") return listenerState.message;
      return "NyxID 已连接";
    case "authorizing":
      return `等待确认 ${view.userCode}`;
    case "checking":
      return "正在连接 NyxID";
    case "unavailable":
      return "桌面版可用";
    case "signed_out":
      return "连接 NyxID 后开始";
    case "denied":
    case "expired":
      return view.message;
    case "error":
      return view.error.message;
  }
}

export function ChatPanel({
  companionName,
  messages,
  nyxIdView,
  pending,
  stopping,
  recovering,
  listenerState,
  canStop,
  onBack,
  onConnect,
  onNewConversation,
  onOpenSettings,
  onRetryListener,
  onSend,
  onStop,
}: ChatPanelProps) {
  const [draft, setDraft] = useState("");
  const scroller = useRef<HTMLDivElement>(null);
  const accountConnected = nyxIdView.state === "connected";
  const connected = accountConnected && listenerState.status === "ready";
  const listenerFailed = accountConnected && listenerState.status === "failed";
  const lastMessage = messages.at(-1);
  const mascotState: MascotState = pending
    ? "thinking"
    : lastMessage?.terminalStatus === "failed"
      ? "failed"
      : lastMessage?.role === "assistant" &&
          lastMessage.terminalStatus !== "cancelled" &&
          lastMessage.text
        ? "happy"
        : "idle";

  useEffect(() => {
    scroller.current?.scrollTo({
      top: scroller.current.scrollHeight,
      behavior: pending ? "auto" : "smooth",
    });
  }, [messages, pending]);

  async function submit() {
    const text = draft.trim();
    if (!text || pending || !connected) return;
    setDraft("");
    await onSend(text);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      void submit();
    }
  }

  return (
    <main className="panel chat-panel" aria-labelledby="chat-title">
      <div className="panel-toolbar chat-toolbar">
        <button
          type="button"
          className="bare-icon"
          aria-label="收起对话"
          onClick={onBack}
        >
          <ArrowLeft aria-hidden="true" />
        </button>
        <div className="chat-toolbar-title" data-window-drag-handle>
          <strong id="chat-title">{companionName}</strong>
          <span>{connectionCopy(nyxIdView, listenerState)}</span>
        </div>
        <button
          type="button"
          className="bare-icon"
          aria-label="开始新对话"
          title="开始新对话"
          disabled={pending || messages.length === 0}
          onClick={onNewConversation}
        >
          <SquarePen aria-hidden="true" />
        </button>
      </div>

      <div className="chat-transcript" ref={scroller} aria-live="polite">
        {messages.length === 0 ? (
          <div className="chat-empty">
            <Mascot state={mascotState} size={132} />
            <strong>
              {connected
                ? `和 ${companionName} 说点什么`
                : listenerFailed
                  ? "对话连接失败"
                  : "连接 NyxID"}
            </strong>
            <span>{connectionCopy(nyxIdView, listenerState)}</span>
            {listenerFailed ||
            (!accountConnected && nyxIdView.state !== "authorizing") ? (
              <button
                type="button"
                className="primary-button"
                disabled={nyxIdView.state === "checking"}
                onClick={() =>
                  listenerFailed ? onRetryListener() : void onConnect()
                }
              >
                {listenerFailed ? (
                  <RefreshCw aria-hidden="true" />
                ) : (
                  <Cloud aria-hidden="true" />
                )}
                {listenerFailed ? "重试对话连接" : "连接 NyxID"}
              </button>
            ) : null}
          </div>
        ) : (
          <ol className="chat-message-list">
            {messages.map((message) => (
              <li
                className={`chat-message chat-message--${message.role}${message.terminalStatus === "failed" ? " chat-message--failed" : ""}`}
                key={message.id}
              >
                {message.role === "assistant" ? (
                  <span className="chat-message-mark" aria-hidden="true">
                    <MessageCircle />
                  </span>
                ) : null}
                <div>
                  {message.text ? <p>{message.text}</p> : null}
                  {!message.text && message.pending ? (
                    <span className="chat-thinking">
                      <LoaderCircle
                        className="is-spinning"
                        aria-hidden="true"
                      />
                      {stopping
                        ? "正在停止"
                        : recovering
                          ? "正在确认处理结果"
                          : "正在处理"}
                    </span>
                  ) : null}
                  {message.terminalStatus ? (
                    <span
                      className={`chat-terminal-status chat-terminal-status--${message.terminalStatus}`}
                    >
                      {message.terminalMessage ??
                        (message.terminalStatus === "failed"
                          ? "这次处理没有完成"
                          : "这次处理已经停止")}
                    </span>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>
        )}
      </div>

      <div className="chat-composer">
        <textarea
          value={draft}
          rows={1}
          maxLength={32_768}
          autoFocus={connected}
          disabled={!connected || pending || recovering}
          aria-label="给 NyxID 发送消息"
          placeholder={
            recovering && !pending
              ? "正在恢复对话…"
              : connected
                ? "发消息…"
                : accountConnected
                  ? "正在准备对话…"
                  : "请先连接 NyxID"
          }
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleKeyDown}
        />
        <button
          type="button"
          className={`chat-send-button${pending ? " chat-send-button--stop" : ""}`}
          aria-label={pending ? "停止处理" : "发送"}
          title={pending ? "停止处理" : "发送"}
          disabled={
            pending
              ? !canStop || stopping
              : !connected || recovering || !draft.trim()
          }
          onClick={() => void (pending ? onStop() : submit())}
        >
          {stopping || (pending && !canStop) ? (
            <LoaderCircle className="is-spinning" aria-hidden="true" />
          ) : pending ? (
            <Square aria-hidden="true" />
          ) : (
            <Send aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          className="chat-settings-button"
          aria-label="打开设置"
          title="设置"
          onClick={onOpenSettings}
        >
          <Settings aria-hidden="true" />
        </button>
      </div>
    </main>
  );
}
