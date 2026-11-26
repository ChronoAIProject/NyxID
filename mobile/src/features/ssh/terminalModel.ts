import { z } from "zod";

export const terminalSize = (cols: number, rows: number) => ({
  cols: Math.max(
    10,
    Math.min(500, Math.round(Number.isFinite(cols) ? cols : 80)),
  ),
  rows: Math.max(
    2,
    Math.min(200, Math.round(Number.isFinite(rows) ? rows : 24)),
  ),
});
export function terminalUrl(
  apiBase: string,
  id: string,
  principal: string,
  cols = 80,
  rows = 24,
) {
  const url = new URL(
    `${apiBase.replace(/\/+$/, "")}/ssh/${encodeURIComponent(id)}/terminal`,
  );
  if (!["http:", "https:"].includes(url.protocol))
    throw new Error("Invalid API URL");
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.search = new URLSearchParams({
    principal,
    ...Object.fromEntries(
      Object.entries(terminalSize(cols, rows)).map(([key, value]) => [
        key,
        String(value),
      ]),
    ),
  }).toString();
  return url.href;
}
export const terminalKeys = [
  "Esc",
  "Tab",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "|",
  "~",
  "/",
  "-",
] as const;
export type TerminalKey = (typeof terminalKeys)[number];
export const TERMINAL_PASTE_MAX_CHARS = 65_536;

/** Truncate UTF-16 clipboard text without leaving a dangling surrogate. */
export function truncateTerminalPaste(text: string): {
  text: string;
  truncated: boolean;
} {
  if (text.length <= TERMINAL_PASTE_MAX_CHARS)
    return { text, truncated: false };
  let value = text.slice(0, TERMINAL_PASTE_MAX_CHARS);
  const last = value.charCodeAt(value.length - 1);
  if (last >= 0xd800 && last <= 0xdbff) value = value.slice(0, -1);
  return { text: value, truncated: true };
}
export function terminalKeyInput(
  key: TerminalKey,
  applicationCursorKeys: boolean,
): string {
  const arrow: Record<string, string> = {
    ArrowUp: "A",
    ArrowDown: "B",
    ArrowRight: "C",
    ArrowLeft: "D",
  };
  return arrow[key]
    ? `\x1b${applicationCursorKeys ? "O" : "["}${arrow[key]}`
    : key === "Esc"
      ? "\x1b"
      : key === "Tab"
        ? "\t"
        : key;
}
export function controlInput(input: string): string {
  if (input.length !== 1) return input;
  const code = input.toUpperCase().charCodeAt(0);
  return code >= 64 && code <= 95
    ? String.fromCharCode(code - 64)
    : input === " "
      ? "\0"
      : input;
}
export const terminalBridgeSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }).strict(),
  z.object({ type: z.literal("input"), data: z.string().max(65548) }).strict(),
  z
    .object({
      type: z.literal("resize"),
      cols: z.number().int().min(10).max(500),
      rows: z.number().int().min(2).max(200),
    })
    .strict(),
  z
    .object({ type: z.literal("ack"), id: z.number().int().positive() })
    .strict(),
]);
export const terminalCommandSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("output"),
      data: z
        .string()
        .max(87384)
        .regex(/^[A-Za-z0-9+/]*={0,2}$/),
      id: z.number().int().positive(),
    })
    .strict(),
  z.object({ type: z.literal("key"), key: z.enum(terminalKeys) }).strict(),
  z
    .object({
      type: z.literal("paste"),
      text: z.string().max(TERMINAL_PASTE_MAX_CHARS),
    })
    .strict(),
  z.object({ type: z.literal("reset") }).strict(),
  z
    .object({
      type: z.literal("session-ended"),
      message: z.string().max(8192),
    })
    .strict(),
  z.object({ type: z.literal("fit") }).strict(),
]);
export type TerminalCommand = z.infer<typeof terminalCommandSchema>;
export function terminalScript(command: TerminalCommand): string {
  return `window.nyxTerminal(${JSON.stringify(
    terminalCommandSchema.parse(command),
  )
    .replace(/</g, "\\u003c")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029")});true;`;
}
export const terminalUpgradeError = (
  status?: number,
  waitingForApproval = false,
): string =>
  status === 401
    ? "Your session expired. Sign in again to open the terminal."
    : status === 403
      ? waitingForApproval
        ? "The server refused this session. An approval may have been denied or expired, or you may no longer have access. Reconnect to try again."
        : "You don't have permission to open this terminal."
      : status === 404
        ? "This SSH connection is no longer available."
        : status === 429
          ? "Too many terminal sessions. Wait a moment and reconnect."
          : status && status >= 500
            ? "The terminal server is unavailable. Try reconnecting."
            : "Couldn't open the terminal connection. Check your network and reconnect.";
export function terminalUpgradeStatus(event: {
  message?: string;
  status?: number;
}) {
  return (
    event.status ??
    (Number(
      event.message?.match(/(?:response|status)[^\d]{0,40}(\d{3})\b/i)?.[1],
    ) ||
      undefined)
  );
}
export function inlineTerminalNavigation() {
  let initial = true;
  const shouldStart = (request: {
    url: string;
    navigationType?: string;
    isTopFrame?: boolean;
  }) => {
    const allowed =
      initial &&
      request.url === "about:blank" &&
      request.isTopFrame !== false &&
      (!request.navigationType || request.navigationType === "other");
    initial = false;
    return allowed;
  };
  return Object.assign(shouldStart, {
    loaded: () => {
      initial = false;
    },
  });
}
