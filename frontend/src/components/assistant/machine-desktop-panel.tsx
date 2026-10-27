import { useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { ChatHeaderCompactContext } from "./chat-header-context";
import {
  ExternalLink,
  Maximize2,
  Minimize2,
  ChevronDown,
  Monitor,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useAppForm } from "@/components/ui/form";
import { useMachineContexts } from "@/hooks/use-machine-access";
import { conversationDesktopKey, type MachineDesktopMetadata } from "@/hooks/use-machines";
import { useNodes } from "@/hooks/use-nodes";
import { cn } from "@/lib/utils";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  desktopFrame,
  desktopInput,
  desktopUrl,
  desktopPoint,
  desktopKey,
  desktopActivity,
  type DesktopActivity,
} from "@/lib/machine-desktop";

/** Live (green), needs attention or connecting (amber), disconnected (red). */
export type DesktopStatus = "live" | "attention" | "offline";

const STATUS_LABEL: Record<DesktopStatus, string> = {
  live: "Live",
  attention: "Needs attention",
  offline: "Disconnected",
};

const STATUS_DOT: Record<DesktopStatus, string> = {
  live: "bg-success",
  attention: "bg-warning",
  offline: "bg-destructive",
};

function StatusDot({ status, label, pulse = false }: { readonly status: DesktopStatus; readonly label: string; readonly pulse?: boolean }) {
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn("h-2 w-2 shrink-0 rounded-full", STATUS_DOT[status], pulse && "motion-safe:animate-pulse")}
    />
  );
}

export function MachineDesktopPanel({
  nodeId,
  conversationId,
  reason,
  name,
  turnActive,
  initialDisplay = "secure",
  initialContextId,
  pane = false,
  leading,
  onStatus,
}: {
  readonly nodeId: string;
  readonly conversationId?: string;
  readonly reason?: string | null;
  readonly name?: string;
  readonly turnActive?: boolean;
  readonly initialDisplay?: "secure" | "dev";
  readonly initialContextId?: string | null;
  /** Docked in the machine column: always shown, never minimised. */
  readonly pane?: boolean;
  /**
   * Opens the toolbar row in place of the machine's name (the machine tabs
   * when there are several). Undefined shows the name; null shows nothing.
   */
  readonly leading?: ReactNode;
  /** Reports the connection state so a tab can show it. */
  readonly onStatus?: (status: DesktopStatus) => void;
}) {
  const collapseKey = conversationId && !pane
    ? `nyxid:desktop-collapsed:${conversationId}:${nodeId}:${initialContextId ?? "legacy"}`
    : null;
  const [storedCollapsed, setCollapsed] = useState(() => {
    try {
      return collapseKey ? localStorage.getItem(collapseKey) === "1" : false;
    } catch {
      return false;
    }
  });
  const collapsed = !pane && storedCollapsed;
  const [fullscreen, setFullscreen] = useState(false);
  useEffect(() => {
    const sync = () => setFullscreen(Boolean(panel.current) && document.fullscreenElement === panel.current);
    document.addEventListener("fullscreenchange", sync);
    return () => document.removeEventListener("fullscreenchange", sync);
  }, []);
  const [activityAt, setActivityAt] = useState(0);
  const [machineName, setMachineName] = useState(name);
  const collapse = useCallback(
    (value: boolean) => {
      setCollapsed(value);
      if (collapseKey) {
        try {
          localStorage.setItem(collapseKey, value ? "1" : "0");
        } catch {
          /* Storage may be disabled. */
        }
      }
    },
    [collapseKey],
  );
  const canvas = useRef<HTMLCanvasElement>(null);
  const panel = useRef<HTMLElement>(null);
  const socket = useRef<WebSocket | null>(null);
  const session = useRef("");
  const sequence = useRef(0n);
  const lastMove = useRef(0);
  const pointer = useRef<{
    x: number;
    y: number;
    at: number;
    id: number;
    button: string;
  } | null>(null);
  const [display, setDisplay] = useState<"secure" | "dev">(initialDisplay);
  const [contextId, setContextId] = useState<string | undefined>(
    initialContextId ?? undefined,
  );
  useEffect(() => {
    setContextId(initialContextId ?? undefined);
  }, [initialContextId]);
  const [attempt, setAttempt] = useState(0);
  const [connected, setConnected] = useState(false);
  const [controls, setControls] = useState(false);
  const [controller, setController] = useState("agent");
  const [action, setAction] = useState("");
  const [error, setError] = useState<string>();
  const wasActive = useRef(Boolean(turnActive));
  useEffect(() => {
    if (turnActive) {
      wasActive.current = true;
      return;
    }
    if (pane || !wasActive.current || controls || controller === "requested") return;
    const timer = setTimeout(
      () => collapse(true),
      Math.max(0, 3000 - (Date.now() - activityAt)),
    );
    return () => clearTimeout(timer);
  }, [pane, turnActive, activityAt, controls, controller, collapse]);
  const status: DesktopStatus = !connected
    ? error
      ? "offline"
      : "attention"
    : error || reason || controller === "requested" || controller === "owner"
      ? "attention"
      : "live";
  const statusText = !connected
    ? error
      ? "Disconnected"
      : "Connecting…"
    : controls
      ? "You are in control"
      : controller === "owner"
        ? "Owner in control in another tab"
        : reason || controller === "requested"
          ? "Needs you"
          : "Watching the agent";
  useEffect(() => {
    onStatus?.(status);
  }, [onStatus, status]);
  const contexts = useMachineContexts(nodeId);
  const control = useCallback((type: string, note?: string) => {
    if (socket.current?.readyState === WebSocket.OPEN)
      socket.current.send(JSON.stringify({ type, note }));
  }, []);
  useEffect(() => {
    const target = canvas.current;
    const ws = new WebSocket(
      desktopUrl(nodeId, conversationId, display, contextId),
    );
    session.current = "";
    sequence.current = 0n;
    pointer.current = null;
    ws.binaryType = "arraybuffer";
    socket.current = ws;
    let disposed = false;
    let lastFrame = 0n;
    let queued = 0;
    let chain = Promise.resolve();
    let refreshAt = -Infinity;
    const surface = document.createElement("canvas");
    let hasFrame = false;
    const refresh = () => {
      if (
        performance.now() - refreshAt > 1000 &&
        ws.readyState === WebSocket.OPEN
      ) {
        refreshAt = performance.now();
        ws.send(JSON.stringify({ type: "refresh_frame" }));
      }
    };
    let activity: DesktopActivity | undefined;
    const paint = () => {
      if (disposed || !target || !hasFrame) return;
      target.width = surface.width;
      target.height = surface.height;
      const context = target.getContext("2d");
      if (!context) return;
      context.drawImage(surface, 0, 0);
      if (activity?.cursor) {
        const x = activity.cursor.x * target.width,
          y = activity.cursor.y * target.height;
        context.beginPath();
        context.arc(x, y, 9, 0, Math.PI * 2);
        context.fillStyle = "#895bffc0";
        context.fill();
        context.strokeStyle = "#ffffff";
        context.lineWidth = 2;
        context.stroke();
      }
    };
    ws.onopen = () => {
      if (!disposed) {
        setConnected(false);
        setControls(false);
        setAction("");
      }
    };
    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        const update = desktopActivity(event.data, session.current);
        if (update) {
          activity = update;
          setAction(update.tool.replaceAll("_", " "));
          setActivityAt(Date.now());
          paint();
          return;
        }
        const frame = desktopFrame(event.data, session.current);
        if (!frame || frame.sequence <= lastFrame) return;
        if (queued >= 4) {
          refresh();
          return;
        }
        queued++;
        chain = chain
          .then(async () => {
            if (disposed) return;
            if (
              frame.rectangle &&
              frame.rectangle.base !== 0n &&
              frame.rectangle.base !== lastFrame
            ) {
              refresh();
              return;
            }
            const next = await createImageBitmap(frame.image);
            try {
              if (disposed) return;
              const rect = frame.rectangle;
              if (
                rect &&
                (next.width !== rect.patchWidth ||
                  next.height !== rect.patchHeight)
              ) {
                refresh();
                return;
              }
              if (!rect || rect.base === 0n) {
                surface.width = rect?.width ?? next.width;
                surface.height = rect?.height ?? next.height;
              }
              surface
                .getContext("2d")
                ?.drawImage(next, rect?.x ?? 0, rect?.y ?? 0);
              hasFrame = true;
              lastFrame = frame.sequence;
              paint();
            } finally {
              next.close();
            }
          })
          .catch(() => {
            if (!disposed) {
              setError("The desktop frame could not be displayed.");
              refresh();
            }
          })
          .finally(() => {
            queued--;
          });
      } else if (typeof event.data === "string") {
        try {
          const message = JSON.parse(event.data) as Record<string, unknown>;
          if (
            message.type === "connected" &&
            typeof message.session_id === "string"
          ) {
            if (typeof message.name === "string") setMachineName(message.name);
            session.current = message.session_id;
            sequence.current = 0n;
            setConnected(true);
            setControls(false);
            setError(undefined);
          }
          if (message.type === "state" || message.type === "connected") {
            if (message.controller !== "agent") {
              activity = undefined;
              setAction("");
              paint();
            }
            setControls(message.controls === true);
            setController(
              typeof message.controller === "string"
                ? message.controller
                : "agent",
            );
          }
          if (message.type === "error")
            setError(
              typeof message.message === "string"
                ? message.message
                : "Desktop unavailable",
            );
        } catch {
          setError("The desktop connection returned an invalid message.");
        }
      }
    };
    ws.onclose = () => {
      if (!disposed) {
        setConnected(false);
        setControls(false);
        setError("Desktop disconnected. Reconnect to continue.");
      }
    };
    ws.onerror = () => {
      if (!disposed)
        setError(
          "Desktop unavailable. Check that the machine is online and computer permissions are enabled.",
        );
    };
    return () => {
      disposed = true;
      surface.width = 0;
      surface.height = 0;
      socket.current = null;
      ws.onopen = null;
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.close();
      target?.getContext("2d")?.clearRect(0, 0, target.width, target.height);
    };
  }, [nodeId, conversationId, attempt, display, contextId]);
  const input = useCallback(
    (tool: string, args: Record<string, unknown>) => {
      if (
        !controls ||
        socket.current?.readyState !== WebSocket.OPEN ||
        socket.current.bufferedAmount > 65536
      )
        return;
      if (session.current)
        socket.current.send(
          desktopInput(session.current, ++sequence.current, tool, args),
        );
    },
    [controls],
  );
  const point = (event: { clientX: number; clientY: number }) => {
    const target = canvas.current;
    return target
      ? desktopPoint(
          target.getBoundingClientRect(),
          target.width,
          target.height,
          event.clientX,
          event.clientY,
        )
      : undefined;
  };
  useEffect(() => {
    const target = canvas.current;
    if (!target || !controls) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const position = desktopPoint(
        target.getBoundingClientRect(),
        target.width,
        target.height,
        event.clientX,
        event.clientY,
      );
      if (!position) return;
      const horizontal = Math.abs(event.deltaX) > Math.abs(event.deltaY);
      input("scroll", {
        ...position,
        direction: horizontal
          ? event.deltaX > 0
            ? "right"
            : "left"
          : event.deltaY > 0
            ? "down"
            : "up",
        amount: 3,
      });
    };
    target.addEventListener("wheel", wheel, { passive: false });
    return () => target.removeEventListener("wheel", wheel);
  }, [controls, input]);
  return (
    <section
      ref={panel}
      className={cn(
        "space-y-3 text-12",
        !pane && "rounded-xl border border-border/50 bg-card p-3",
        fullscreen && "overflow-auto bg-background p-4",
      )}
      aria-label="Live machine desktop"
      data-private="true"
      data-ph-no-capture
    >
      {/* One row when it fits. Otherwise the machine tabs keep the first row
          and the view controls move, together and still on one line, to a
          second; a narrow row shrinks the workspace picker. */}
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {leading === undefined ? (
          <span className="flex min-w-0 shrink items-center gap-2">
            <StatusDot status={status} label={statusText} />
            <span className="min-w-0 truncate font-medium">
              {machineName ?? name ?? nodeId}
            </span>
          </span>
        ) : leading}
        {/* Connecting and disconnected show on the screen; once live, say who drives. */}
        {connected ? <span className="sr-only" aria-live="polite">{statusText}</span> : null}
        <div className="flex min-w-0 flex-1 basis-60 items-center gap-2">
        <div
          role="group"
          aria-label="Desktop display"
          title={controls ? "Hand back to switch displays" : undefined}
          className="flex shrink-0 rounded-lg border border-hairline p-0.5"
        >
          {(["secure", "dev"] as const).map((value) => (
            <Button
              key={value}
              size="sm"
              variant="ghost"
              aria-pressed={display === value}
              aria-label={value === "secure" ? "Secure browser" : "Dev browser"}
              title={value === "secure" ? "Secure browser" : "Dev browser"}
              disabled={controls && display !== value}
              className={cn(
                "h-6 min-h-0 px-2 text-11",
                display === value ? "bg-overlay-strong text-foreground" : "text-muted-foreground",
              )}
              onClick={() => {
                setConnected(false);
                setDisplay(value);
              }}
            >
              {value === "secure" ? "Secure" : "Dev"}
            </Button>
          ))}
        </div>
        {conversationId && !pane ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 shrink-0 px-0"
            aria-label={collapsed ? "Show desktop" : "Minimise desktop"}
            aria-expanded={!collapsed}
            onClick={() => collapse(!collapsed)}
          >
            {collapsed ? <ChevronDown /> : <Minimize2 />}
          </Button>
        ) : null}
        {collapsed && action ? (
          <span className="min-w-0 truncate text-muted-foreground">{action}</span>
        ) : null}
        {/* Never narrower than a shrunken picker (64px) plus the two buttons;
            the machine tabs scroll before anything here is cut off. */}
        <div className="ml-auto flex min-w-32 shrink items-center gap-1">
          <Select
            value={contextId ?? "legacy"}
            onValueChange={(value) => {
              setConnected(false);
              setContextId(value === "legacy" ? undefined : value);
            }}
            disabled={controls}
          >
            <SelectTrigger
              aria-label="Desktop context"
              title={contexts.isError ? "Separated contexts are unavailable on this machine" : "Workspace and browser"}
              className="h-7 w-auto min-w-16 max-w-40 shrink basis-40 text-11 [&>span]:min-w-0 [&>span]:truncate"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="legacy">Shared legacy desktop</SelectItem>
              {(contexts.data ?? []).map((context) => (
                <SelectItem key={context.context_id} value={context.context_id}>
                  {context.agent_name} · {context.actor_label}
                  {context.group_id ? " · group task" : ""} · generation {context.generation}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {contexts.isError ? (
            <span role="status" className="sr-only">Separated contexts are unavailable on this machine.</span>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 shrink-0 px-0"
            aria-label={fullscreen ? "Exit fullscreen" : "Expand desktop"}
            title={fullscreen ? "Exit fullscreen" : "Fullscreen"}
            onClick={() => {
              if (fullscreen) void document.exitFullscreen();
              else void panel.current?.requestFullscreen();
            }}
          >
            {fullscreen ? <Minimize2 /> : <Maximize2 />}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 shrink-0 px-0"
            aria-label="Pop out desktop"
            title="Open in a new window"
            onClick={() =>
              window.open(
                `/assistant/machines/${encodeURIComponent(nodeId)}/desktop?display=${display}${conversationId ? `&conversation_id=${encodeURIComponent(conversationId)}` : ""}${contextId ? `&context_id=${encodeURIComponent(contextId)}` : ""}`,
                "_blank",
                "noopener,noreferrer",
              )
            }
          >
            <ExternalLink />
          </Button>
        </div>
        </div>
      </div>
      <div hidden={collapsed} className="relative">
        <canvas
          ref={canvas}
          width={1280}
          height={800}
          tabIndex={controls ? 0 : -1}
          aria-label="Machine screen. Take control to use mouse and keyboard."
          className={`block ${pane ? "" : "max-h-[65vh] "}w-full rounded-xl bg-background object-contain focus:outline-none focus:ring-1 focus:ring-primary ${controls ? "cursor-crosshair" : "cursor-default"}`}
          onPointerMove={(event) => {
            const position = point(event);
            if (
              position &&
              !pointer.current &&
              performance.now() - lastMove.current > 40
            ) {
              lastMove.current = performance.now();
              input("move_cursor", position);
            }
          }}
          onPointerDown={(event) => {
            const position = point(event);
            if (!controls || !position) return;
            event.preventDefault();
            event.currentTarget.focus();
            event.currentTarget.setPointerCapture(event.pointerId);
            pointer.current = {
              ...position,
              at: performance.now(),
              id: event.pointerId,
              button:
                event.button === 2
                  ? "right"
                  : event.button === 1
                    ? "middle"
                    : "left",
            };
          }}
          onPointerUp={(event) => {
            const start = pointer.current;
            pointer.current = null;
            if (!start || start.id !== event.pointerId) return;
            event.currentTarget.releasePointerCapture(event.pointerId);
            const end = point(event);
            if (!end) return;
            if (Math.hypot(end.x - start.x, end.y - start.y) > 4)
              input("drag", {
                from_x: start.x,
                from_y: start.y,
                to_x: end.x,
                to_y: end.y,
                button: start.button,
                duration_ms: Math.min(
                  10000,
                  Math.round(performance.now() - start.at),
                ),
              });
            else input("click", { ...end, button: start.button, count: 1 });
          }}
          onPointerCancel={() => {
            pointer.current = null;
          }}
          onDoubleClick={(event) => {
            const position = point(event);
            if (position)
              input("click", { ...position, button: "left", count: 2 });
          }}
          onContextMenu={(event) => {
            if (controls) event.preventDefault();
          }}
          onCompositionEnd={(event) => {
            if (controls && event.data)
              input("type_text", { text: event.data });
          }}
          onPaste={(event) => {
            if (controls) {
              event.preventDefault();
              input("type_text", {
                text: event.clipboardData.getData("text/plain").slice(0, 8192),
              });
            }
          }}
          onKeyDown={(event) => {
            if (
              !controls ||
              event.nativeEvent.isComposing ||
              event.key === "Shift" ||
              event.key === "Control" ||
              event.key === "Alt" ||
              event.key === "Meta"
            )
              return;
            if (
              (event.metaKey || event.ctrlKey) &&
              event.key.toLowerCase() === "v"
            )
              return;
            event.preventDefault();
            const modifiers = [
              event.ctrlKey && "CTRL",
              event.metaKey && "META",
              event.altKey && "ALT",
              event.shiftKey && "SHIFT",
            ].filter(Boolean);
            if (
              event.key.length === 1 &&
              !event.ctrlKey &&
              !event.metaKey &&
              !event.altKey
            )
              input("type_text", { text: event.key });
            else if (modifiers.length)
              input("hotkey", { keys: [...modifiers, desktopKey(event.key)] });
            else input("press_key", { key: desktopKey(event.key) });
          }}
        />
        {/* What is wrong, or what is needed, shows on the screen itself. */}
        {!connected || error ? (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 rounded-xl bg-background/70 p-4 text-center backdrop-blur-[2px] dark:bg-black/60">
            {error ? (
              <>
                <p role="alert" className="max-w-sm text-destructive">{error}</p>
                {!connected ? (
                  <Button size="sm" onClick={() => setAttempt((value) => value + 1)}>Reconnect</Button>
                ) : null}
              </>
            ) : (
              <p className="flex items-center gap-2 text-muted-foreground">
                <span className="h-1.5 w-1.5 rounded-full bg-warning motion-safe:animate-pulse" aria-hidden="true" />
                Connecting…
              </p>
            )}
            {reason ? <p role="status" className="max-w-sm text-warning">NyxBot needs you on this machine: {reason}</p> : null}
          </div>
        ) : null}
        {connected && !error && (reason || controller === "requested") && !controls ? (
          <p role="status" className="absolute inset-x-3 top-3 rounded-lg bg-warning/15 px-3 py-2 text-warning backdrop-blur-sm light:bg-warning light:text-white">
            NyxBot needs you on this machine: {reason ?? "Please take control."}
          </p>
        ) : null}
      </div>
      {!collapsed && action && !controls && controller === "agent" ? (
        <p className="text-muted-foreground" aria-live="polite">
          Agent action: {action}
        </p>
      ) : null}
      <DesktopControls
        connected={connected}
        controls={controls}
        controller={controller}
        control={control}
      />
      {controls ? (
        <p className="text-muted-foreground">
          The agent cannot use this machine while you control it. Screen frames
          and input are not recorded.
        </p>
      ) : null}
    </section>
  );
}
function DesktopControls({
  connected,
  controls,
  controller,
  control,
}: {
  readonly connected: boolean;
  readonly controls: boolean;
  readonly controller: string;
  readonly control: (type: string, note?: string) => void;
}) {
  const form = useAppForm({ defaultValues: { note: "" } });
  return (
    <div className="flex min-w-0 items-center justify-end gap-2">
      {!controls ? (
        <Button
          size="sm"
          className="shrink-0"
          variant="primary"
          disabled={!connected || controller === "taking"}
          onClick={() => control("take_control")}
        >
          Take control
        </Button>
      ) : (
        <form
          className="flex min-w-0 flex-1 gap-2"
          onSubmit={form.handleSubmit(({ note }) => {
            control("hand_back", note);
            form.reset();
          })}
        >
          <Input
            className="h-8"
            aria-label="Hand-back note"
            placeholder="Optional note, e.g. logged in"
            maxLength={2000}
            {...form.register("note")}
          />
          <Button type="submit" size="sm" className="shrink-0" variant="primary">
            Hand back
          </Button>
        </form>
      )}
      <Button
        size="sm"
        className="shrink-0"
        variant="destructive"
        disabled={!connected}
        onClick={() => control("stop")}
      >
        Stop
      </Button>
    </div>
  );
}

/**
 * A conversation's open desktops. One is shown at a time; with several, tabs
 * switch between them and mark the ones the agent is using or that need you.
 * Only the shown desktop streams.
 */
function MachineDesktopSwitcher({
  conversationId,
  desktops,
  turnActive,
  pane,
}: {
  readonly conversationId: string;
  readonly desktops: readonly MachineDesktopMetadata[];
  readonly turnActive?: boolean;
  readonly pane: boolean;
}) {
  const nodes = useNodes();
  const [selected, setSelected] = useState<string>();
  const [shownStatus, setShownStatus] = useState<DesktopStatus>("attention");
  const shown = desktops.find((desktop) => conversationDesktopKey(desktop) === selected) ?? desktops[0];
  if (!shown) return null;
  const nodeName = (id: string) => nodes.data?.find((node) => node.id === id)?.name;
  const label = (desktop: MachineDesktopMetadata) => {
    const sameNode = desktops.filter((other) => other.node_id === desktop.node_id);
    const parts = [nodeName(desktop.node_id) ?? desktop.node_id];
    if (sameNode.some((other) => (other.display ?? "secure") !== (desktop.display ?? "secure"))) {
      parts.push(desktop.display === "dev" ? "Dev browser" : "Secure browser");
    }
    if (sameNode.some((other) => other.context_id !== desktop.context_id) && desktop.context_id) {
      parts.push(`context ${desktop.context_id.slice(0, 6)}`);
    }
    return parts.join(" · ");
  };
  const tabs = desktops.length > 1 ? (
    <div role="tablist" aria-label="Machines" className="flex min-w-0 max-w-full shrink-0 gap-1 overflow-x-auto">
      {desktops.map((desktop) => {
        const key = conversationDesktopKey(desktop);
        const active = desktop === shown;
        const live = turnActive && desktop.status === "agent";
        return (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => setSelected(key)}
            className={cn(
              "flex h-7 max-w-56 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-12 transition-colors",
              active ? "bg-overlay-strong text-foreground" : "text-muted-foreground hover:bg-overlay hover:text-foreground",
            )}
          >
            <Monitor aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{label(desktop)}</span>
            {active ? (
              <StatusDot status={shownStatus} label={STATUS_LABEL[shownStatus]} pulse={shownStatus === "live" && live} />
            ) : desktop.reason ? (
              <StatusDot status="attention" label="Needs you" />
            ) : live ? (
              <StatusDot status="live" label="Agent working" pulse />
            ) : null}
          </button>
        );
      })}
    </div>
  ) : null;
  return (
    <div className={cn("flex min-h-0 flex-col", pane && "h-full")}>
      {/* In the column, the machine is one card in a stack; other cards
          (pinned messages, for example) can sit beside it later. */}
      <div className={cn(pane && "assistant-scrollbar min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain py-3 pl-3")}>
        {/* Glued to the slab's right wall: square on that side. */}
        <div className={cn(pane && "rounded-l-2xl bg-card p-3 shadow-sm")}>
        <MachineDesktopPanel
          key={conversationDesktopKey(shown)}
          nodeId={shown.node_id}
          name={nodeName(shown.node_id)}
          conversationId={conversationId}
          reason={shown.reason}
          initialDisplay={shown.display}
          initialContextId={shown.context_id}
          turnActive={turnActive}
          pane={pane}
          leading={tabs}
          onStatus={setShownStatus}
        />
        </div>
      </div>
    </div>
  );
}

/** A conversation's open desktops, inline above the transcript (narrow screens). */
export function ConversationMachineDesktops({
  conversationId,
  desktops,
  turnActive,
}: {
  readonly conversationId: string;
  readonly desktops: readonly MachineDesktopMetadata[];
  readonly turnActive?: boolean;
}) {
  return desktops.length ? (
    <div className="mx-auto w-full max-w-[758px] px-4 pb-3">
      <MachineDesktopSwitcher conversationId={conversationId} desktops={desktops} turnActive={turnActive} pane={false} />
    </div>
  ) : null;
}

/**
 * The machine column docked beside the chat. The shell's divider sets its
 * width; the chat's Machine button hides it. Each desktop can still go
 * fullscreen or pop out.
 */
export function MachinePane({
  conversationId,
  desktops,
  turnActive,
}: {
  readonly conversationId: string;
  readonly desktops: readonly MachineDesktopMetadata[];
  readonly turnActive?: boolean;
}) {
  return <MachineDesktopSwitcher conversationId={conversationId} desktops={desktops} turnActive={turnActive} pane />;
}

/** The thread's machine button: shows or hides its desktops. */
export function MachinePaneToggle({
  open,
  live,
  onToggle,
}: {
  readonly open: boolean;
  /** The agent is working on it right now. */
  readonly live: boolean;
  readonly onToggle: () => void;
}) {
  const compact = useContext(ChatHeaderCompactContext);
  return (
    <Button
      size="sm"
      variant="ghost"
      aria-label={open ? "Hide machine" : "Show machine"}
      aria-pressed={open}
      title={open ? "Hide machine" : "Show machine"}
      className={cn("relative", compact ? "h-7 w-7 px-0" : undefined, open && "bg-overlay-strong text-foreground")}
      onClick={onToggle}
    >
      <Monitor aria-hidden="true" />
      <span className={compact ? "sr-only" : undefined}>Machine</span>
      {live ? (
        <span aria-hidden="true" className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-success motion-safe:animate-pulse" />
      ) : null}
    </Button>
  );
}
