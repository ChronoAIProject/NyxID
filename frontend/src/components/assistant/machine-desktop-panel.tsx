import { useCallback, useEffect, useRef, useState } from "react";
import { ExternalLink, Maximize2, Monitor } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useAppForm } from "@/components/ui/form";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useMachineDesktops } from "@/hooks/use-machines";
import {
  desktopFrame,
  desktopInput,
  desktopUrl,
  desktopPoint,
  desktopKey,
  desktopActivity,
  type DesktopActivity,
} from "@/lib/machine-desktop";

export function MachineDesktopPanel({
  nodeId,
  conversationId,
  reason,
}: {
  readonly nodeId: string;
  readonly conversationId?: string;
  readonly reason?: string | null;
}) {
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
  const [attempt, setAttempt] = useState(0);
  const [connected, setConnected] = useState(false);
  const [controls, setControls] = useState(false);
  const [controller, setController] = useState("agent");
  const [action, setAction] = useState("");
  const [error, setError] = useState<string>();
  const { data: config } = usePublicConfig();
  const control = useCallback((type: string, note?: string) => {
    if (socket.current?.readyState === WebSocket.OPEN)
      socket.current.send(JSON.stringify({ type, note }));
  }, []);
  useEffect(() => {
    const target = canvas.current;
    const ws = new WebSocket(
      desktopUrl(nodeId, conversationId, config?.node_ws_url),
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
  }, [nodeId, conversationId, config?.node_ws_url, attempt]);
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
      className="space-y-3 rounded-xl border border-border/50 bg-card p-3 text-[12px]"
      aria-label="Live machine desktop"
      data-private="true"
      data-ph-no-capture
    >
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="size-4" />
        <span className="font-medium">Live desktop</span>
        <span className="text-muted-foreground">
          {!connected
            ? "Connecting…"
            : controls
              ? "You are in control"
              : controller === "owner"
                ? "Owner in control in another tab"
                : "Watching the agent"}
        </span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Expand desktop"
          onClick={() => {
            void panel.current?.requestFullscreen();
          }}
        >
          <Maximize2 />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Pop out desktop"
          onClick={() =>
            window.open(
              `/machines/${encodeURIComponent(nodeId)}/desktop${conversationId ? `?conversation_id=${encodeURIComponent(conversationId)}` : ""}`,
              "_blank",
              "noopener,noreferrer",
            )
          }
        >
          <ExternalLink />
        </Button>
      </div>
      {(reason || controller === "requested") && !controls ? (
        <p role="status" className="rounded-lg border border-warning/30 p-3">
          NyxBot needs you on this machine: {reason ?? "Please take control."}
        </p>
      ) : null}
      <canvas
        ref={canvas}
        width={1280}
        height={800}
        tabIndex={controls ? 0 : -1}
        aria-label="Machine screen. Take control to use mouse and keyboard."
        className={`block max-h-[65vh] w-full rounded-lg bg-background object-contain focus:outline-none focus:ring-1 focus:ring-primary ${controls ? "cursor-crosshair" : "cursor-default"}`}
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
          if (controls && event.data) input("type_text", { text: event.data });
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
      {action && !controls && controller === "agent" ? (
        <p className="text-muted-foreground" aria-live="polite">
          Agent action: {action}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      ) : null}
      <DesktopControls
        connected={connected}
        controls={controls}
        controller={controller}
        control={control}
        reconnect={() => setAttempt((value) => value + 1)}
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
  reconnect,
}: {
  readonly connected: boolean;
  readonly controls: boolean;
  readonly controller: string;
  readonly control: (type: string, note?: string) => void;
  readonly reconnect: () => void;
}) {
  const form = useAppForm({ defaultValues: { note: "" } });
  return (
    <div className="flex flex-wrap items-center gap-2">
      {!connected ? (
        <Button onClick={() => reconnect()}>Reconnect</Button>
      ) : null}
      {!controls ? (
        <Button
          variant="primary"
          disabled={!connected || controller === "taking"}
          onClick={() => control("take_control")}
        >
          Take control
        </Button>
      ) : (
        <form
          className="flex flex-1 gap-2"
          onSubmit={form.handleSubmit(({ note }) => {
            control("hand_back", note);
            form.reset();
          })}
        >
          <Input
            aria-label="Hand-back note"
            placeholder="Optional note, e.g. logged in"
            maxLength={2000}
            {...form.register("note")}
          />
          <Button type="submit" variant="primary">
            Hand back
          </Button>
        </form>
      )}
      <Button
        variant="destructive"
        disabled={!connected}
        onClick={() => control("stop")}
      >
        Stop
      </Button>
    </div>
  );
}

export function ConversationMachineDesktops({
  conversationId,
}: {
  readonly conversationId: string;
}) {
  const machines = useMachineDesktops(conversationId);
  return machines.data?.length ? (
    <div className="mx-auto w-full max-w-[758px] space-y-3 px-4 pb-3">
      {machines.data.map((machine) => (
        <MachineDesktopPanel
          key={machine.node_id}
          nodeId={machine.node_id}
          conversationId={conversationId}
          reason={machine.reason}
        />
      ))}
    </div>
  ) : null;
}
