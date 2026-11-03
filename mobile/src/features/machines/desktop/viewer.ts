import { desktopActivity, desktopFrame, desktopInput, desktopInputValue, type DesktopFrame } from './protocol';

export type ViewerState = { controls?: boolean; taking?: boolean; phase: 'connecting' | 'watching' | 'attention' | 'reconnecting' | 'unavailable'; label: string };
export type ViewSocket = {
  readyState: number;
  binaryType: string;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: { code: number }) => void) | null;
  onerror: (() => void) | null;
  bufferedAmount?: number;
  send(value: string | ArrayBuffer): void;
  close(): void;
};
type Clock = {
  now(): number;
  random(): number;
  later(callback: () => void, milliseconds: number): unknown;
  cancel(timer: unknown): void;
};
const systemClock: Clock = {
  now: () => Date.now(), random: () => Math.random(),
  later: (callback, ms) => setTimeout(callback, ms), cancel: timer => clearTimeout(timer as ReturnType<typeof setTimeout>),
};
// These are server authority/runtime refusals, not session logout signals.
export function terminalDesktopFailure(cause: unknown): boolean {
  const error = cause as { statusCode?: number; errorCode?: number; error_code?: number } | null;
  return !!error && ([400, 401, 403, 404, 405, 410, 426].includes(error.statusCode ?? 0) ||
    [8000, 12400, 12401, 12408, 12409, 12413, 12415, 12416, 12419, 12420, 12421].includes(error.errorCode ?? error.error_code ?? 0));
}
export function refusalLabel(cause: unknown): string {
  const error = cause as { errorCode?: unknown; error_code?: unknown; message?: unknown } | null;
  const code = error?.errorCode ?? error?.error_code;
  const known: Record<number, string> = {
    12408: 'The machine owner is controlling this desktop. Wait for them to hand it back.',
    12420: 'Your permission to use this desktop was revoked.',
    12421: 'Desktop authority is stale. Retry after permissions refresh.',
    12400: 'Desktop capability is disabled on this machine.',
    12401: 'You do not have permission to control this desktop.',
    12419: 'This machine needs an update before opening this desktop.',
  };
  if (typeof code === 'number' && known[code]) return known[code];
  if (typeof error?.message === 'string' && error.message.trim()) return error.message.slice(0, 500);
  return `Desktop permission or runtime unavailable${typeof code === 'number' && Number.isInteger(code) ? ` (code ${code})` : ''}.`;
}

let stopActiveViewer: (() => void) | undefined;

/** Owns the socket/control grant. Renderer input cannot acquire control.
 * A reconnect clears the explicit human intent and requires another takeover. */
export function createDesktopViewer(deps: {
  current(): boolean;
  connect(): Promise<ViewSocket>;
  state(value: ViewerState): void;
  frame(value: DesktopFrame): void;
  clear(): void;
  invalidated?(): void;
  clock?: Clock;
}) {
  stopActiveViewer?.();
  const clock = deps.clock ?? systemClock;
  let stopped = false;
  let socket: ViewSocket | undefined;
  let session = '';
  let controls = false, taking = false, intent = false, sequence = 0n;
  let inputWindow = 0, inputCount = 0;
  let attempt = 0;
  let lastLive = 0;
  let refreshAt = -Infinity;
  let retryTimer: unknown;
  let watchTimer: unknown;
  let generation = 0;
  const active = () => !stopped && deps.current();
  const returnControl = () => {
    if (intent && (controls || taking) && socket?.readyState === 1) {
      try { socket.send('{"type":"hand_back","note":""}'); } catch { /* Best effort before clearing the grant. */ }
    }
    controls = false; taking = false; intent = false;
  };
  const detach = () => {
    generation++;
    const previous = socket;
    // Returning an explicitly acquired control lease never stops the task.
    returnControl();
    socket = undefined; session = ''; controls = false; taking = false; intent = false; sequence = 0n;
    clock.cancel(retryTimer); clock.cancel(watchTimer);
    if (previous) {
      previous.onopen = null; previous.onmessage = null; previous.onclose = null; previous.onerror = null;
      previous.close();
    }
    deps.clear();
  };
  const stop = () => { stopped = true; detach(); if (stopActiveViewer === stop) stopActiveViewer = undefined; };
  const requireActive = () => {
    if (active()) return true;
    const invalidated = !stopped && !deps.current();
    stop();
    if (invalidated) deps.invalidated?.();
    return false;
  };
  stopActiveViewer = stop;
  const unavailable = (label: string) => {
    detach(); stopped = true;
    if (stopActiveViewer === stop) stopActiveViewer = undefined;
    deps.state({ phase: 'unavailable', label });
  };
  const refresh = () => {
    if (!requireActive()) return;
    if (session && socket?.readyState === 1 && clock.now() - refreshAt >= 1000) {
      refreshAt = clock.now();
      // Do not expose a generic send method to UI or the renderer.
      try { socket.send('{"type":"refresh_frame"}'); } catch { reconnect(); }
    }
  };
  const reconnect = () => {
    detach();
    if (!requireActive()) return;
    if (attempt >= 4) { unavailable('Desktop disconnected. Retry to reconnect.'); return; }
    const delay = Math.min(8000, 1000 * 2 ** attempt++) * (0.75 + clock.random() * 0.5);
    deps.state({ phase: 'reconnecting', label: 'Reconnecting to the desktop…' });
    retryTimer = clock.later(() => void connect(), delay);
  };
  const watch = () => {
    if (!requireActive()) return;
    // Session/state messages prove liveness; an idle desktop may send no JPEGs.
    if (clock.now() - lastLive >= 30_000) { reconnect(); return; }
    watchTimer = clock.later(watch, 100);
  };
  const connect = async () => {
    if (!requireActive()) return;
    deps.state({ phase: 'connecting', label: 'Connecting to the desktop…' });
    const expected = generation;
    lastLive = clock.now(); watch();
    try {
      const ws = await deps.connect();
      if (!requireActive() || generation !== expected) { ws.close(); return; }
      socket = ws; ws.binaryType = 'arraybuffer';
      const owned = () => active() && socket === ws;
      ws.onopen = () => { requireActive(); };
      ws.onmessage = event => {
        if (!requireActive() || socket !== ws) return;
        if (event.data instanceof ArrayBuffer) {
          if (!session) return;
          if (desktopActivity(event.data, session)) return;
          const frame = desktopFrame(event.data, session);
          if (frame) deps.frame(frame); else refresh();
          return;
        }
        if (typeof event.data !== 'string' || event.data.length > 16_384) return;
        try {
          const value = JSON.parse(event.data) as Record<string, unknown>;
          if (value.type === 'error') {
            returnControl();
            if (terminalDesktopFailure(value)) unavailable(refusalLabel(value));
            else deps.state({ phase: 'attention', controls: false, label: refusalLabel(value) });
            return;
          }
          if (value.type === 'connected') {
            if (typeof value.session_id !== 'string' || !/^[\da-f]{8}(-[\da-f]{4}){3}-[\da-f]{12}$/i.test(value.session_id)) {
              unavailable('The server returned unsupported desktop data.'); return;
            }
            deps.clear(); session = value.session_id; controls = false; intent = false; taking = false; sequence = 0n;
          }
          if (session && (value.type === 'state' || value.type === 'connected')) {
            lastLive = clock.now();
            controls = intent && value.controls === true && value.controller === 'owner';
            if (controls) taking = false;
            if (value.type === 'state' && value.controls === false && value.controller !== 'taking') { intent = false; taking = false; }
            const attention = value.controller === 'owner' || value.controller === 'requested' || !!value.reason;
            deps.state({ phase: attention ? 'attention' : 'watching', controls, taking, label: controls
              ? 'You are controlling this desktop' : value.controller === 'owner' ? 'Owner is controlling elsewhere'
              : taking ? 'Taking control…' : attention ? 'Needs owner attention' : 'Watching the desktop' });
          }
        } catch { unavailable('The server returned unsupported desktop data.'); }
      };
      ws.onclose = event => {
        if (!owned()) return;
        // Unknown upgrade refusals are terminal until the human chooses Retry.
        if (!session || [1000, 1002, 1003, 1008, 1010].includes(event.code)) unavailable('Desktop unavailable. Retry to reconnect.');
        else reconnect();
      };
      ws.onerror = () => {
        if (!owned()) return;
        if (!session) unavailable('The server could not open this desktop.');
        else reconnect();
      };
    } catch (cause) {
      if (!requireActive() || generation !== expected) return;
      if (terminalDesktopFailure(cause)) unavailable(refusalLabel(cause));
      else reconnect();
    }
  };
  const takeControl = () => {
    if (!active() || !session || socket?.readyState !== 1 || controls || taking) return;
    intent = true; taking = true;
    deps.state({ phase: 'attention', controls: false, taking: true, label: 'Taking control…' });
    try { socket.send('{"type":"take_control"}'); } catch { reconnect(); }
  };
  const handBack = () => {
    if (!intent || (!controls && !taking)) return;
    returnControl();
    deps.state({ phase: 'watching', controls: false, label: 'Returning control…' });
  };
  // True means queued on this socket, not acknowledged or executed remotely.
  const input = (value: unknown): boolean => {
    if (!requireActive()) return false;
    if (!intent || !controls || !session || socket?.readyState !== 1 || (socket.bufferedAmount ?? 0) > 65536) return false;
    const command = desktopInputValue(value);
    if (!command) return false;
    if (clock.now() - inputWindow >= 1000) { inputWindow = clock.now(); inputCount = 0; }
    if (inputCount >= 60) return false;
    let frame: ArrayBuffer;
    try { frame = desktopInput(session, sequence + 1n, command); }
    catch { return false; /* Invalid/oversized input never leaves the client. */ }
    try {
      socket.send(frame); inputCount++; sequence++;
      return true;
    } catch {
      // Delivery is unknown; clear authority and never replay the input.
      reconnect();
      return false;
    }
  };
  const stopTask = () => {
    if (!active() || !intent || !controls || socket?.readyState !== 1) return;
    try { socket.send('{"type":"stop"}'); } catch { reconnect(); }
  };
  return { start: () => void connect(), stop, refresh, takeControl, handBack, input, stopTask, fail: unavailable };
}
