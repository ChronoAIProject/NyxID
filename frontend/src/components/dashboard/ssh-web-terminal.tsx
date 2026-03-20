import { useEffect, useRef, useCallback, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

type ConnectionStatus = "connecting" | "connected" | "disconnected" | "error";

interface ControlMessage {
  readonly type: string;
  readonly message?: string;
}

interface SshWebTerminalProps {
  readonly serviceId: string;
  readonly principal: string;
  readonly nodeWsUrl?: string;
  readonly onDisconnect?: () => void;
}

function buildWebSocketUrl(
  serviceId: string,
  principal: string,
  cols: number,
  rows: number,
  nodeWsUrl?: string,
): string {
  // Derive the backend WS base from the node WebSocket URL (which points to the backend).
  // Falls back to same-origin (works when frontend is served by the backend).
  let wsBase: string;
  if (nodeWsUrl) {
    try {
      const parsed = new URL(nodeWsUrl);
      parsed.pathname = "";
      parsed.search = "";
      parsed.hash = "";
      wsBase = parsed.toString().replace(/\/$/, "");
    } catch {
      wsBase = window.location.origin.replace(/^https:/, "wss:").replace(/^http:/, "ws:");
    }
  } else {
    wsBase = window.location.origin.replace(/^https:/, "wss:").replace(/^http:/, "ws:");
  }
  const params = new URLSearchParams({
    principal,
    cols: String(cols),
    rows: String(rows),
  });
  return `${wsBase}/api/v1/ssh/${encodeURIComponent(serviceId)}/terminal?${params.toString()}`;
}

export function SshWebTerminal({
  serviceId,
  principal,
  nodeWsUrl,
  onDisconnect,
}: SshWebTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  const [status, setStatus] = useState<ConnectionStatus>("connecting");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const cleanup = useCallback(() => {
    if (resizeTimerRef.current !== null) {
      clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = null;
    }

    const ws = wsRef.current;
    if (ws !== null) {
      ws.onopen = null;
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.close();
      }
      wsRef.current = null;
    }

    const term = terminalRef.current;
    if (term !== null) {
      // Clean up resize observers
      const observer = (term as unknown as Record<string, unknown>)._nyxidResizeObserver;
      if (observer instanceof ResizeObserver) {
        observer.disconnect();
      }
      const windowHandler = (term as unknown as Record<string, unknown>)._nyxidWindowResizeHandler;
      if (typeof windowHandler === "function") {
        window.removeEventListener("resize", windowHandler as EventListener);
      }
      term.dispose();
      terminalRef.current = null;
    }

    fitAddonRef.current = null;
  }, []);

  const connect = useCallback(() => {
    if (!containerRef.current) return;

    cleanup();

    setStatus("connecting");
    setErrorMessage(null);

    const terminal = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Menlo, Monaco, 'Courier New', monospace",
      fontSize: 14,
      lineHeight: 1.2,
      scrollback: 5000,
      theme: {
        background: "#0f172a",
        foreground: "#e2e8f0",
        cursor: "#c76a34",
        selectionBackground: "#334155",
        selectionForeground: "#f8fafc",
        black: "#1e293b",
        red: "#ef4444",
        green: "#22c55e",
        yellow: "#eab308",
        blue: "#3b82f6",
        magenta: "#a855f7",
        cyan: "#06b6d4",
        white: "#e2e8f0",
        brightBlack: "#475569",
        brightRed: "#f87171",
        brightGreen: "#4ade80",
        brightYellow: "#facc15",
        brightBlue: "#60a5fa",
        brightMagenta: "#c084fc",
        brightCyan: "#22d3ee",
        brightWhite: "#f8fafc",
      },
    });

    terminalRef.current = terminal;

    const fitAddon = new FitAddon();
    fitAddonRef.current = fitAddon;
    terminal.loadAddon(fitAddon);

    terminal.open(containerRef.current);

    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        webglAddon.dispose();
      });
      terminal.loadAddon(webglAddon);
    } catch {
      // WebGL not available; fall back to canvas renderer (default).
    }

    fitAddon.fit();

    // Re-fit on window resize and container resize
    const handleWindowResize = () => {
      requestAnimationFrame(() => {
        fitAddon.fit();
      });
    };
    window.addEventListener("resize", handleWindowResize);

    if (containerRef.current) {
      const resizeObserver = new ResizeObserver(() => {
        requestAnimationFrame(() => {
          fitAddon.fit();
        });
      });
      resizeObserver.observe(containerRef.current);
      // Store for cleanup
      (terminal as unknown as Record<string, unknown>)._nyxidResizeObserver = resizeObserver;
      (terminal as unknown as Record<string, unknown>)._nyxidWindowResizeHandler = handleWindowResize;
    }

    const cols = terminal.cols;
    const rows = terminal.rows;

    terminal.writeln("\x1b[90mConnecting...\x1b[0m");

    const wsUrl = buildWebSocketUrl(serviceId, principal, cols, rows, nodeWsUrl);
    const ws = new WebSocket(wsUrl);
    ws.binaryType = "arraybuffer";
    wsRef.current = ws;

    ws.onopen = () => {
      // Connection opened; wait for server "connected" message before
      // updating status to "connected".
    };

    ws.onmessage = (event: MessageEvent) => {
      if (!mountedRef.current) return;

      if (event.data instanceof ArrayBuffer) {
        const data = new Uint8Array(event.data);
        terminal.write(data);
        return;
      }

      if (typeof event.data === "string") {
        try {
          const msg = JSON.parse(event.data) as ControlMessage;

          if (msg.type === "connected") {
            setStatus("connected");
            terminal.clear();
            terminal.focus();
            // Re-fit to ensure terminal fills available space
            if (fitAddonRef.current) {
              requestAnimationFrame(() => {
                fitAddonRef.current?.fit();
              });
            }
            return;
          }

          if (msg.type === "error") {
            setStatus("error");
            setErrorMessage(msg.message ?? "Unknown error");
            terminal.writeln(`\r\n\x1b[31mError: ${msg.message ?? "Unknown error"}\x1b[0m`);
            return;
          }

          if (msg.type === "closed") {
            setStatus("disconnected");
            terminal.writeln("\r\n\x1b[90mSession closed by remote host.\x1b[0m");
            onDisconnect?.();
            return;
          }
        } catch {
          // Not valid JSON; ignore.
        }
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      setStatus((prev) => {
        if (prev !== "error") {
          terminal.writeln("\r\n\x1b[90mConnection closed.\x1b[0m");
          return "disconnected";
        }
        return prev;
      });
    };

    ws.onerror = () => {
      if (!mountedRef.current) return;
      setStatus("error");
      setErrorMessage("WebSocket connection failed");
      terminal.writeln("\r\n\x1b[31mWebSocket connection failed.\x1b[0m");
    };

    // Send terminal input as binary frames.
    terminal.onData((data: string) => {
      if (ws.readyState === WebSocket.OPEN) {
        const encoder = new TextEncoder();
        ws.send(encoder.encode(data));
      }
    });

    // Debounced resize handler.
    terminal.onResize(({ cols: newCols, rows: newRows }: { cols: number; rows: number }) => {
      if (resizeTimerRef.current !== null) {
        clearTimeout(resizeTimerRef.current);
      }
      resizeTimerRef.current = setTimeout(() => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "resize", cols: newCols, rows: newRows }));
        }
        resizeTimerRef.current = null;
      }, 100);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serviceId, principal, nodeWsUrl, cleanup]);

  // Initial connection on mount. Guard against React Strict Mode double-mount.
  const connectedOnceRef = useRef(false);
  useEffect(() => {
    mountedRef.current = true;
    if (!connectedOnceRef.current) {
      connectedOnceRef.current = true;
      connect();
    }

    return () => {
      mountedRef.current = false;
      cleanup();
      connectedOnceRef.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serviceId, principal]);

  // Handle window resize to refit the terminal.
  useEffect(() => {
    function handleResize() {
      const fitAddon = fitAddonRef.current;
      if (fitAddon !== null) {
        try {
          fitAddon.fit();
        } catch {
          // Container might not be visible yet.
        }
      }
    }

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  // Observe container size changes for more reliable resizing.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver(() => {
      const fitAddon = fitAddonRef.current;
      if (fitAddon !== null) {
        try {
          fitAddon.fit();
        } catch {
          // Ignore if not yet attached.
        }
      }
    });

    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      {/* Status bar */}
      <div className="flex items-center gap-2 border-b border-border/50 bg-[#0f172a] px-3 py-1.5">
        <div
          className={`h-2 w-2 rounded-full ${
            status === "connected"
              ? "bg-emerald-500 shadow-[0_0_6px_rgba(34,197,94,0.5)]"
              : status === "connecting"
                ? "bg-yellow-500 animate-pulse"
                : status === "error"
                  ? "bg-red-500"
                  : "bg-zinc-500"
          }`}
        />
        <span className="text-xs text-slate-400">
          {status === "connecting" && "Connecting..."}
          {status === "connected" && "Connected"}
          {status === "disconnected" && "Disconnected"}
          {status === "error" && (errorMessage ?? "Error")}
        </span>
        {(status === "disconnected" || status === "error") && (
          <button
            type="button"
            onClick={connect}
            className="ml-auto rounded px-2 py-0.5 text-xs text-slate-300 transition-colors hover:bg-slate-700 hover:text-white"
          >
            Reconnect
          </button>
        )}
      </div>

      {/* Terminal container */}
      <div ref={containerRef} className="flex-1 overflow-hidden bg-[#0f172a] p-1" />
    </div>
  );
}
