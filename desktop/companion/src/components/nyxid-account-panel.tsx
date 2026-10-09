import {
  CircleAlert,
  Cloud,
  ExternalLink,
  LoaderCircle,
  LogIn,
  LogOut,
  RefreshCw,
  X,
} from "lucide-react";

import type { NyxIdView } from "../runtime";
import { IconButton } from "./icon-button";

type RetryAction = Extract<
  NyxIdView,
  { state: "error" }
>["error"]["retryAction"];

interface HeaderAction {
  readonly kind: "connect" | "cancel" | "refresh" | "logout";
  readonly label: string;
  readonly run: () => Promise<void>;
}

interface NyxIdAccountPanelProps {
  readonly view: NyxIdView;
  readonly pending: boolean;
  readonly onConnect: () => Promise<void>;
  readonly onCancel: () => Promise<void>;
  readonly onRefresh: () => Promise<void>;
  readonly onLogout: () => Promise<void>;
  readonly onOpenAssistant: () => Promise<void>;
}

function statusCopy(view: NyxIdView): string {
  switch (view.state) {
    case "unavailable":
      return "请在桌面版中连接";
    case "checking":
      return "正在确认账号";
    case "signed_out":
      return "未连接";
    case "authorizing":
      return "等待浏览器确认";
    case "connected":
      return view.user.email;
    case "denied":
    case "expired":
      return view.message;
    case "error":
      return view.error.message;
  }
}

function accountInitial(view: Extract<NyxIdView, { state: "connected" }>) {
  const source = view.user.displayName?.trim() || view.user.email;
  return Array.from(source)[0]?.toLocaleUpperCase() ?? "N";
}

function retryHeaderAction(
  retryAction: RetryAction,
  handlers: Pick<
    NyxIdAccountPanelProps,
    "onConnect" | "onCancel" | "onRefresh" | "onLogout"
  >,
): HeaderAction | undefined {
  switch (retryAction) {
    case "connect":
      return { kind: "connect", label: "重试连接", run: handlers.onConnect };
    case "cancel":
      return { kind: "cancel", label: "重试取消", run: handlers.onCancel };
    case "refresh":
      return { kind: "refresh", label: "重试刷新", run: handlers.onRefresh };
    case "logout":
      return { kind: "logout", label: "重试断开", run: handlers.onLogout };
    case null:
      return undefined;
  }
}

function actionIcon(kind: HeaderAction["kind"]) {
  switch (kind) {
    case "connect":
      return <LogIn aria-hidden="true" />;
    case "cancel":
      return <X aria-hidden="true" />;
    case "refresh":
      return <RefreshCw aria-hidden="true" />;
    case "logout":
      return <LogOut aria-hidden="true" />;
  }
}

export function NyxIdAccountPanel({
  view,
  pending,
  onConnect,
  onCancel,
  onRefresh,
  onLogout,
  onOpenAssistant,
}: NyxIdAccountPanelProps) {
  const terminal =
    view.state === "denied" ||
    view.state === "expired" ||
    view.state === "error";
  const headerAction: HeaderAction | undefined =
    view.state === "signed_out"
      ? { kind: "connect", label: "连接", run: onConnect }
      : view.state === "denied" || view.state === "expired"
        ? { kind: "connect", label: "重新连接", run: onConnect }
        : view.state === "error"
          ? retryHeaderAction(view.error.retryAction, {
              onConnect,
              onCancel,
              onRefresh,
              onLogout,
            })
          : undefined;

  return (
    <section
      className="settings-section nyxid-account"
      aria-labelledby="nyxid-account-title"
      aria-busy={pending || view.state === "checking"}
    >
      <div className="nyxid-account-header">
        {view.state === "connected" ? (
          <span className="nyxid-avatar" aria-hidden="true">
            {accountInitial(view)}
          </span>
        ) : (
          <span className="nyxid-mark" aria-hidden="true">
            <Cloud />
          </span>
        )}
        <div className="nyxid-account-heading">
          <h2 id="nyxid-account-title">
            {view.state === "connected"
              ? view.user.displayName?.trim() || "NyxID"
              : "NyxID"}
          </h2>
          <p>{statusCopy(view)}</p>
        </div>

        {view.state === "checking" ? (
          <LoaderCircle
            className="nyxid-spinner is-spinning"
            aria-hidden="true"
          />
        ) : null}
        {headerAction ? (
          <button
            type="button"
            className="inline-action"
            disabled={pending}
            onClick={() => void headerAction.run()}
          >
            {actionIcon(headerAction.kind)}
            {headerAction.label}
          </button>
        ) : null}
        {view.state === "authorizing" ? (
          <IconButton
            className="nyxid-header-action"
            label="取消连接"
            disabled={pending}
            onClick={() => void onCancel()}
          >
            <X aria-hidden="true" />
          </IconButton>
        ) : null}
      </div>

      {view.state === "authorizing" ? (
        <div className="nyxid-approval" role="status">
          <span>确认码</span>
          <code aria-label="NyxID 登录确认码">{view.userCode}</code>
          <small>请在已打开的浏览器中核对并确认</small>
        </div>
      ) : null}

      {terminal ? (
        <div className="nyxid-problem" role="alert">
          <CircleAlert aria-hidden="true" />
          <span>{statusCopy(view)}</span>
        </div>
      ) : null}

      {view.state === "connected" ? (
        <>
          <div className="nyxid-capability-counts" aria-label="NyxID 服务状态">
            <div>
              <strong>{view.capabilities.enabledCount}</strong>
              <span>已启用</span>
            </div>
            <div>
              <strong>{view.capabilities.attentionCount}</strong>
              <span>需处理</span>
            </div>
            <div>
              <strong>{view.capabilities.disabledCount}</strong>
              <span>已停用</span>
            </div>
          </div>

          {view.capabilities.services.length > 0 ? (
            <ul className="nyxid-service-list" aria-label="NyxID 服务">
              {view.capabilities.services.slice(0, 3).map((service) => (
                <li key={service.id}>
                  <span
                    className={`nyxid-service-state nyxid-service-state--${service.state}`}
                    aria-hidden="true"
                  />
                  <span>{service.label}</span>
                  <small>
                    {service.state === "enabled"
                      ? "已启用"
                      : service.state === "disabled"
                        ? "已停用"
                        : "需处理"}
                  </small>
                </li>
              ))}
              {view.capabilities.services.length > 3 ? (
                <li className="nyxid-service-more">
                  还有 {view.capabilities.services.length - 3} 项
                </li>
              ) : null}
            </ul>
          ) : (
            <p className="nyxid-empty">还没有连接外部服务</p>
          )}

          <div className="nyxid-account-actions">
            <IconButton
              label="刷新 NyxID 服务状态"
              disabled={pending}
              onClick={() => void onRefresh()}
            >
              <RefreshCw
                className={pending ? "is-spinning" : ""}
                aria-hidden="true"
              />
            </IconButton>
            <IconButton
              label="打开 NyxID Assistant"
              disabled={pending}
              onClick={() => void onOpenAssistant()}
            >
              <ExternalLink aria-hidden="true" />
            </IconButton>
            <IconButton
              label="断开 NyxID"
              disabled={pending}
              onClick={() => void onLogout()}
            >
              <LogOut aria-hidden="true" />
            </IconButton>
          </div>
        </>
      ) : null}
    </section>
  );
}
