import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useRef, useState } from "react";
import type RFB from "@novnc/novnc";
import { Eye, Maximize, MousePointer2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCmaDesktop } from "@/hooks/use-assistant-cma-agents";
import { useAuthStore } from "@/stores/auth-store";

const reasons: Record<string, string> = {
  desktop_busy:
    "Another viewer is connected to this Sandbox. Close that viewer before reconnecting.",
  agent_stopped:
    "This Agent is stopped. Its shared desktop connection is unavailable.",
  agent_deleted:
    "This Agent was deleted. Its desktop connection is unavailable.",
  agent_failed: "This Agent has failed. Its desktop connection is unavailable.",
  agent_paused: "This Agent is paused. Its desktop connection is unavailable.",
  agent_state_unknown:
    "This Agent’s current state could not be confirmed. Check its status before reconnecting.",
  agent_unavailable:
    "This Agent’s current state does not allow a desktop connection.",
  machine_not_included:
    "This environment has no desktop capability. Opening this view does not create one.",
  desktop_not_included:
    "This environment has no desktop capability. Opening this view does not create one.",
  desktop_not_enabled:
    "This environment has no desktop capability. Opening this view does not create one.",
  sandbox_retired:
    "The parent environment is retired. Its desktop connection is unavailable.",
  sandbox_retiring:
    "The parent environment is being retired. Its desktop connection is unavailable.",
  sandbox_paused:
    "The parent environment is paused. Resume it from Sandboxes before opening its desktop.",
  sandbox_unavailable:
    "The parent environment is not running. Check its status in Sandboxes.",
  authorization_expired:
    "Desktop authorization expired. Refresh to verify your current access.",
  isolated_configuration:
    "This Agent’s isolation configuration does not support desktop access.",
};

export function CmaDesktopPanel({ agentId }: { readonly agentId: string }) {
  useUiTranslation();
  const descriptor = useCmaDesktop(agentId);
  const [attempt, setAttempt] = useState(0);
  const expectedPath = `/api/v1/assistant/nyxagent/cma-agents/${encodeURIComponent(agentId)}/desktop/ws`;
  const data = descriptor.data;
  const valid = data?.available && data.websocket_path === expectedPath;
  return (
    <section aria-label={translateUi("Shared Sandbox desktop")} className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="type-secondary-emphasis">{translateUi("Shared Sandbox desktop")}</h2>
        <Button
          variant="outline"
          isLoading={descriptor.isFetching}
          onClick={() => {
            void descriptor
              .refetch()
              .then(() => setAttempt((value) => value + 1));
          }}
        >
          <RefreshCw />{translateUi("Reconnect")}</Button>
      </div>
      <p className="type-secondary text-text-tertiary">{translateUi("This desktop belongs to the parent Sandbox and may be shared by several Agents and chats. Only one viewer can connect at a time. Manual input does not pause the Agent.")}</p>
      {descriptor.isPending && (
        <p role="status">{translateUi("Checking desktop capability…")}</p>
      )}
      {descriptor.error && (
        <p role="alert" className="text-12 text-destructive">
          {descriptor.error.message}
        </p>
      )}
      {data && !data.available && (
        <p
          role="status"
          className="rounded-lg border border-dashed border-border p-8 text-13 text-text-tertiary"
        >
          {reasons[data.reason ?? ""] ??
            translateUi("Desktop unavailable{{value1}}. Refresh to check the current state.", { value1: data.reason ? ` (${data.reason})` : "" })}
        </p>
      )}
      {data?.available && !valid && (
        <p role="alert">{translateUi("The desktop relay address could not be verified.")}</p>
      )}
      {valid && !descriptor.error && (
        <RfbViewer key={`${agentId}:${attempt}`} path={expectedPath} />
      )}
    </section>
  );
}

function RfbViewer({ path }: { readonly path: string }) {
  useUiTranslation();
  const owner = useAuthStore((state) => state.user?.id);
  const screen = useRef<HTMLDivElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const connection = useRef<RFB | null>(null);
  const [state, setState] = useState<
    "connecting" | "connected" | "disconnected"
  >("connecting");
  const [viewOnly, setViewOnly] = useState(true);
  const [error, setError] = useState<string>();
  useEffect(() => {
    if (!owner) return;
    let disposed = false;
    let client: RFB | undefined;
    const failed = (message: string) => {
      if (!disposed) {
        setState("disconnected");
        setError(message);
        setViewOnly(true);
      }
    };
    const deadline = window.setTimeout(() => {
      failed("The desktop connection timed out. Reconnect to try again.");
      client?.disconnect();
    }, 30_000);
    void import("@novnc/novnc")
      .then(({ default: Client }) => {
        if (
          disposed ||
          !screen.current ||
          useAuthStore.getState().user?.id !== owner
        )
          return;
        const url = new URL(path, window.location.origin);
        url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
        client = new Client(screen.current, url.toString());
        client.viewOnly = true;
        client.scaleViewport = true;
        client.resizeSession = false;
        connection.current = client;
        client.addEventListener("connect", () => {
          window.clearTimeout(deadline);
          if (!disposed) setState("connected");
        });
        client.addEventListener("disconnect", () => {
          window.clearTimeout(deadline);
          failed(
            "Desktop connection ended. Refresh authorization and reconnect. Another viewer may already be connected.",
          );
        });
        client.addEventListener("securityfailure", () => {
          failed(
            "Desktop authentication was refused. Refresh to verify access.",
          );
          client?.disconnect();
        });
        client.addEventListener("credentialsrequired", () => {
          failed(
            "CMA desktop requires credentials that cannot be supplied by this viewer.",
          );
          client?.disconnect();
        });
      })
      .catch(() =>
        failed("Unable to load the desktop viewer. Reconnect to try again."),
      );
    return () => {
      disposed = true;
      window.clearTimeout(deadline);
      connection.current = null;
      if (client) {
        client.viewOnly = true;
        client.disconnect();
      }
    };
  }, [path, owner]);
  function toggleInput() {
    const next = !viewOnly;
    if (!connection.current || state !== "connected") return;
    connection.current.viewOnly = next;
    setViewOnly(next);
    if (!next) connection.current.focus();
  }
  return (
    <div
      ref={surface}
      className="rounded-xl border border-hairline bg-background p-3"
    >
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <span role="status" className="mr-auto text-12 text-text-tertiary">
          {state === "connected"
            ? translateUi("Connected")
            : state === "connecting"
              ? translateUi("Connecting…")
              : translateUi("Disconnected")}
        </span>
        <Button
          disabled={state !== "connected"}
          aria-pressed={!viewOnly}
          onClick={toggleInput}
        >
          {viewOnly ? <Eye /> : <MousePointer2 />}
          {viewOnly ? translateUi("View only") : translateUi("Input enabled")}
        </Button>
        <Button
          aria-label={translateUi("Fullscreen desktop")}
          onClick={() => {
            void surface.current
              ?.requestFullscreen()
              .catch(() =>
                setError("Fullscreen is unavailable in this browser."),
              );
          }}
        >
          <Maximize />{translateUi("Fullscreen")}</Button>
      </div>
      {error && (
        <p role="alert" className="mb-3 text-12 text-destructive">
          {error}
        </p>
      )}
      <div
        ref={screen}
        className="aspect-video min-h-64 w-full overflow-hidden rounded-lg bg-muted"
      />
      <p className="mt-3 type-secondary text-text-tertiary">{translateUi("Connections expire after 30 minutes. Reconnect verifies current access and starts in view-only mode.")}</p>
    </div>
  );
}
