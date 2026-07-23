// Connect-marker encoding: how an assistant message carries "the user needs
// to connect service X" through a text-only transport.
//
// Aevatar's chat history returns one `content` string per message, and the
// FE may only call chat + history — there is no third endpoint to rehydrate
// card state from on reload. So anything not carried inside the message text
// does not survive a reload. The marker is a transport encoding for that
// constraint, derived from the `eanz17/nyxid-chat` prototype
// (`public/blocks.js`).
//
// Two rules make this safe, and both are load-bearing:
//
//  1. **Aevatar writes the marker programmatically**, from its own
//     `GET /api/v1/keys` read — never by prompting the model to emit it. The
//     slug is therefore not model output.
//  2. **Parsing happens once, here, at the transport seam.** Callers get
//     typed `ConnectCardContentBlock`s; no renderer ever sees the raw string.
//     The card then re-resolves every actionable field from the NyxID
//     catalog (see `connect-card.tsx`) — marker fields are display data only.
//
// When Aevatar can emit typed blocks in history, this module is deleted and
// the transport keeps emitting the same blocks. Nothing downstream changes.

import type { ConnectCardContentBlock } from "@/types/assistant";

/**
 * The marker's opening fence: exactly three backticks at **column zero**.
 *
 * Column zero is required, not cosmetic. Aevatar writes this programmatically,
 * so it costs nothing there — while any indentation allowance would (a) make
 * an indented code example mint a card and (b) break `renderableText`'s
 * partial-line guard, which can only cheaply recognise an opener at the start
 * of a line.
 */
const CONNECT_FENCE = /^```[ \t]*nyxid:connect[ \t]*$/;
/**
 * Opening delimiter of any fenced code block: 3+ backticks or 3+ tildes,
 * indented up to three spaces. CommonMark allows that indentation, and a
 * column-zero-only matcher misses ` ```markdown ` — letting a marker quoted
 * inside it escape into live parsing.
 */
const FENCE_OPEN = /^ {0,3}(`{3,}|~{3,})/;
/** A bare run of fence characters — a candidate close. */
const FENCE_CLOSE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/;
/** Longest prefix a partially-streamed marker opener can have. */
const MARKER_OPENER = "```nyxid:connect";

/**
 * Could this partial line still grow into a marker opener?
 *
 * Must track `CONNECT_FENCE` exactly. When the two disagree the streamed
 * projection shrinks: a form the guard doesn't recognise renders as prose,
 * then the parser claims it as a marker and takes it back — which corrupts
 * the transport's suffix diff. `` ``` nyxid:connec `` (space after the
 * backticks) was exactly that gap.
 */
function couldOpenMarker(line: string): boolean {
  if (line.length === 0) return false;
  if (CONNECT_FENCE.test(line)) return true;
  // Collapse the grammar's optional interstitial whitespace before comparing.
  const canonical = line.replace(/^```[ \t]+/, "```");
  return MARKER_OPENER.startsWith(canonical);
}
/** NyxID catalog slugs: `api-github`, `llm-openai`, `lark_bot`. */
const SLUG = /^[a-z0-9][a-z0-9_-]{0,80}$/;

const MAX_REASON_LENGTH = 300;
const MAX_SCOPES = 12;
const MAX_SCOPE_LENGTH = 60;

export interface ConnectMarker {
  readonly catalogSlug: string;
  /** One-line human justification. Display only. */
  readonly reason: string;
  /** Display only in v1 — never an action input (scopes come from catalog). */
  readonly requestedScopes: readonly string[];
}

export type MessageSegment =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "connect"; readonly marker: ConnectMarker }
  /** An unterminated marker at the end of a still-streaming message. */
  | { readonly kind: "pending" };

interface SplitOptions {
  /**
   * Mid-stream: hold back an unterminated trailing marker instead of showing
   * half-written JSON to the user. Off for completed/stored text, where an
   * unterminated fence is just literal text.
   */
  readonly allowPartial?: boolean;
}

/** CRLF/CR → LF. Applied before any line matching. */
function normalizeNewlines(source: string): string {
  return String(source ?? "").replace(/\r\n?/g, "\n");
}

function parseMarker(body: string): ConnectMarker | null {
  let payload: unknown;
  try {
    payload = JSON.parse(body);
  } catch {
    return null;
  }
  if (typeof payload !== "object" || payload === null) return null;
  const record = payload as Record<string, unknown>;
  const rawSlug = record.catalog_slug ?? record.slug;
  const slug = typeof rawSlug === "string" ? rawSlug.trim().toLowerCase() : "";
  if (!SLUG.test(slug)) return null;
  const reason =
    typeof record.reason === "string"
      ? record.reason.replace(/\s+/g, " ").trim().slice(0, MAX_REASON_LENGTH)
      : "";
  const requestedScopes = Array.isArray(record.requested_scopes)
    ? record.requested_scopes
        .filter((scope): scope is string => typeof scope === "string")
        .map((scope) => scope.slice(0, MAX_SCOPE_LENGTH))
        .slice(0, MAX_SCOPES)
    : [];
  return { catalogSlug: slug, reason, requestedScopes };
}

/**
 * Split assistant text into renderable segments.
 *
 * Tracks ordinary fenced-code state, so a `nyxid:connect` line *inside*
 * another code fence — a quoted example, a pasted log, a tool result echoing
 * one back — stays literal text and does not become a live connect card.
 * Ean's splitter has no such state (`public/blocks.js:41-73`), which makes it
 * injectable by anything that can get a fenced block into the transcript.
 *
 * A malformed marker (bad JSON, bad slug) degrades to literal text: better a
 * visible oddity than a card pointing somewhere unintended.
 */
export function splitConnectMarkers(
  source: string,
  { allowPartial = false }: SplitOptions = {},
): readonly MessageSegment[] {
  // Normalise line endings first. A stray `\r` makes the opener fail its
  // exact-match regex while still opening an ordinary fence, so a CRLF
  // transcript would leak raw marker syntax into rendered prose.
  const lines = normalizeNewlines(source).split("\n");
  const segments: MessageSegment[] = [];
  let buffer: string[] = [];
  /** Open ordinary code fence: its delimiter character and opener length. */
  let inOrdinaryFence: { char: string; length: number } | null = null;

  const flushText = (): void => {
    if (buffer.length === 0) return;
    const text = buffer.join("\n");
    if (text.trim()) segments.push({ kind: "text", text });
    buffer = [];
  };

  let index = 0;
  while (index < lines.length) {
    const line = lines[index] ?? "";

    if (inOrdinaryFence !== null) {
      // CommonMark: a fence closes only on the same character, at least as
      // long as the opener. Without this, a ```` ``` ```` line closes a
      // ```` ```` ```` block and everything after it — including a marker the
      // author clearly meant as quoted content — escapes into live parsing.
      const close = FENCE_CLOSE.exec(line);
      if (
        close?.[1] &&
        close[1][0] === inOrdinaryFence.char &&
        close[1].length >= inOrdinaryFence.length
      ) {
        inOrdinaryFence = null;
      }
      buffer.push(line);
      index += 1;
      continue;
    }

    if (CONNECT_FENCE.test(line)) {
      let cursor = index + 1;
      const body: string[] = [];
      let closed = false;
      while (cursor < lines.length) {
        const candidate = FENCE_CLOSE.exec(lines[cursor] ?? "");
        if (candidate?.[1]?.startsWith("```")) {
          closed = true;
          break;
        }
        body.push(lines[cursor] ?? "");
        cursor += 1;
      }
      if (!closed) {
        if (allowPartial) {
          flushText();
          segments.push({ kind: "pending" });
          return segments;
        }
        // Completed text with a dangling fence: literal, not a card.
        buffer.push(line, ...body);
        break;
      }
      const marker = parseMarker(body.join("\n"));
      if (marker) {
        flushText();
        segments.push({ kind: "connect", marker });
      } else {
        buffer.push(line, ...body, "```");
      }
      index = cursor + 1;
      continue;
    }

    const open = FENCE_OPEN.exec(line);
    if (open?.[1]) {
      inOrdinaryFence = { char: open[1][0] as string, length: open[1].length };
      buffer.push(line);
      index += 1;
      continue;
    }

    buffer.push(line);
    index += 1;
  }

  flushText();
  return segments;
}

/** True when the text carries at least one well-formed marker. */
export function hasConnectMarker(source: string): boolean {
  return splitConnectMarkers(source).some(
    (segment) => segment.kind === "connect",
  );
}

/**
 * The prose a partially-streamed message should show right now.
 *
 * Markers are dropped (they become cards at message end) and everything from
 * an unterminated marker onward is withheld, so half-written JSON never
 * reaches the transcript. Grows monotonically as deltas arrive, which lets
 * the transport forward just the newly-safe suffix.
 */
export function renderableText(source: string): string {
  const text = normalizeNewlines(source);
  // Withhold a trailing *partial* line that opens with a backtick: it may
  // still be growing into a marker opener, and "```nyxid:conn" must never
  // flash as prose only to be retracted a delta later. Without this the
  // result is not monotonic, and the transport's suffix diff breaks. The
  // withheld line is released as soon as its newline arrives, and the final
  // text is emitted whole at message end regardless.
  const lastNewline = text.lastIndexOf("\n");
  const trailingLine = text.slice(lastNewline + 1);
  // Withhold the trailing *partial* line only when it could still become a
  // marker opener. Anything else — ordinary inline code, ```` ```ts ```` —
  // streams immediately. Withholding more needlessly stalls prose;
  // withholding less lets a growing opener render and then need retracting.
  const safeSource = couldOpenMarker(trailingLine)
    ? text.slice(0, lastNewline + 1)
    : text;
  return (
    splitConnectMarkers(safeSource, { allowPartial: true })
      .filter((segment) => segment.kind === "text")
      .map((segment) => (segment.kind === "text" ? segment.text : ""))
      .join("\n")
      // Trailing newlines only — never spaces. A marker's opening fence must
      // start a line, so the newline separating prose from it belongs to the
      // text run right up until the fence is recognised, at which point the
      // splitter absorbs it. That one-character shrink would corrupt the
      // transport's suffix diff. Trailing *spaces* are never absorbed and are
      // real prose (a cancelled turn keeps its partial "Hello, "), so they
      // stay.
      .replace(/[\r\n]+$/, "")
  );
}

/**
 * Project a marker into the shipped card block.
 *
 * Everything actionable is left deliberately thin: `service_name` falls back
 * to the slug and `auth_kind` to `api_key`, because `ConnectCard` re-resolves
 * both from the NyxID catalog before rendering an affordance. Filling them in
 * from marker text would make model-adjacent data an action input.
 */
export function connectMarkerToBlock(
  marker: ConnectMarker,
  blockId: string,
): ConnectCardContentBlock {
  return {
    type: "connect_card",
    block_id: blockId,
    catalog_slug: marker.catalogSlug,
    service_name: marker.catalogSlug,
    icon_url: "",
    subtitle: marker.reason || "Required by this request",
    auth_kind: "api_key",
    requested_scopes: [...marker.requestedScopes],
    key_id: null,
    granted_scopes: null,
    device_user_code: null,
    device_verification_url: null,
    state: "needs_connection",
    error_message: null,
    steps: [
      {
        title: `Connect ${marker.catalogSlug}`,
        body: marker.reason || "Connect this service to continue.",
        done: false,
      },
    ],
    footer: "Brokered by NyxID · revoke anytime in AI Services",
    reason_code: "NYXID_SERVICE_NOT_CONNECTED",
  };
}
