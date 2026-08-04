import { z } from "zod";

/**
 * Client-side validation for service-level default request headers
 * (NyxID#356). Mirrors the backend rules in
 * `backend/src/models/default_request_header.rs` so users get instant
 * feedback before the request round-trips.
 *
 * v1 stores values in plaintext — `sensitive` is a UI redaction flag
 * only. Actual secrets (Authorization, Cookie, etc.) are denylisted and
 * must go through the service's `auth_method` instead.
 */

export const MAX_DEFAULT_HEADERS = 16;
export const MAX_HEADER_NAME_LEN = 256;
export const MAX_HEADER_VALUE_LEN = 4096;

/** Exact-match denylist. Case-insensitive. */
const DENYLISTED_HEADER_NAMES: ReadonlyArray<string> = [
  "authorization",
  "cookie",
  "set-cookie",
  "host",
  "content-length",
  "transfer-encoding",
  "connection",
  "upgrade",
  "te",
  "trailer",
  "expect",
  "keep-alive",
  "user-agent",
  "x-real-ip",
  // RFC 7239 replacement for the `x-forwarded-*` family. Blocked here
  // for the same trust-boundary reason — defaults are injected on
  // every proxy path and would let an admin spoof IP / proto / host.
  // Mirror `backend::default_request_header::DENYLISTED_HEADER_NAMES`.
  "forwarded",
];

/** Prefix denylist. Case-insensitive. Any header starting with one of these
 *  is rejected. Mirror `backend::default_request_header::DENYLISTED_HEADER_PREFIXES`. */
const DENYLISTED_HEADER_PREFIXES: ReadonlyArray<string> = [
  "x-nyxid-",
  "x-forwarded-",
  "proxy-",
  // WebSocket handshake metadata is protocol-managed by the WS client
  // libraries on both transports. Overriding `Sec-WebSocket-Key`,
  // `-Version`, `-Extensions`, `-Protocol`, etc. would break or weaken
  // every WS proxy for the service.
  "sec-websocket-",
];

/**
 * RFC 7230 token chars: VCHAR minus delimiters. Matches reqwest's
 * HeaderName parser on the backend.
 */
const HEADER_NAME_TOKEN_RE = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;

/**
 * Placeholder the backend returns in place of a sensitive value. Must
 * stay in sync with `backend::default_request_header::REDACTED_PLACEHOLDER`.
 *
 * The character (U+2022 BULLET) is non-ASCII so it would normally be
 * rejected by the transport-value check below; we make a single
 * exception so a GET → editor → PUT round trip on unrelated rows
 * doesn't force the user to retype every sensitive value. The backend's
 * `reconcile_with_stored` helper swaps the placeholder back to the real
 * stored value before persisting.
 */
export const REDACTED_PLACEHOLDER = "•••••";

export function isDenylistedHeaderName(raw: string): boolean {
  const lower = raw.trim().toLowerCase();
  if (lower.length === 0) return true;
  if (DENYLISTED_HEADER_NAMES.includes(lower)) return true;
  return DENYLISTED_HEADER_PREFIXES.some((p) => lower.startsWith(p));
}

// NOTE: We deliberately avoid `.transform()` on the name/value fields. RHF
// v8's resolver generics break when a Zod schema mixes input-vs-output
// types inside a form that's also used via `useForm<Schema>()`. The
// backend trims whitespace server-side, so the frontend only validates
// the raw string. Any callers that need trimmed values should do so at
// submit time (`value.trim()`).
export const defaultRequestHeaderSchema = z
  .object({
    name: z
      .string()
      .min(1, "Header name is required")
      .max(
        MAX_HEADER_NAME_LEN,
        `Header name must be at most ${String(MAX_HEADER_NAME_LEN)} characters`,
      )
      .refine(
        (v) => v.trim().length > 0,
        "Header name must not be blank",
      )
      .refine(
        (v) => HEADER_NAME_TOKEN_RE.test(v.trim()),
        "Invalid characters in header name (RFC 7230 token chars only)",
      ),
    // Length cap is enforced unconditionally; the byte-level check moves
    // to the object-level superRefine below so it can look at
    // `sensitive` and allow the redaction placeholder to round-trip.
    value: z.string().max(
      MAX_HEADER_VALUE_LEN,
      `Header value must be at most ${String(MAX_HEADER_VALUE_LEN)} characters`,
    ),
    overridable: z.boolean(),
    sensitive: z.boolean(),
  })
  .superRefine((value, ctx) => {
    if (isDenylistedHeaderName(value.name)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["name"],
        message:
          "This header name is reserved. Use the service auth method for secrets (Authorization / Cookie), or pick a non-reserved name.",
      });
    }

    // Transport-value byte check: matches
    // `reqwest::header::HeaderValue::from_str` / backend
    // `is_valid_header_value` — HTAB + visible ASCII only. Reject here
    // so the editor catches bad values before hitting the API.
    //
    // Exception: a `sensitive` row may carry the exact redaction
    // placeholder (U+2022 BULLET × 5) returned by the backend on GET.
    // The server's `reconcile_with_stored` restores the real stored
    // value before persisting; without this exception the user would
    // have to retype every sensitive header on every unrelated edit.
    const isRedactedRoundtrip =
      value.sensitive && value.value === REDACTED_PLACEHOLDER;
    if (
      !isRedactedRoundtrip &&
      // eslint-disable-next-line no-control-regex
      !/^[\x09\x20-\x7e]*$/.test(value.value)
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["value"],
        message:
          "Value may only contain visible ASCII characters, space, or tab (no control bytes, DEL, or non-ASCII characters like é or emoji)",
      });
    }
  });

export type DefaultRequestHeader = z.infer<typeof defaultRequestHeaderSchema>;

/**
 * Validates a complete list: count cap, case-insensitive uniqueness, and
 * each row.
 */
export const defaultRequestHeaderListSchema = z
  .array(defaultRequestHeaderSchema)
  .max(
    MAX_DEFAULT_HEADERS,
    `At most ${String(MAX_DEFAULT_HEADERS)} default headers are allowed`,
  )
  .superRefine((rows, ctx) => {
    const seen = new Map<string, number>();
    rows.forEach((row, idx) => {
      const key = row.name.trim().toLowerCase();
      if (key.length === 0) return;
      const prior = seen.get(key);
      if (prior !== undefined) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: [idx, "name"],
          message: `Duplicate header name (case-insensitive match with row ${String(prior + 1)})`,
        });
      } else {
        seen.set(key, idx);
      }
    });
  });

export type DefaultRequestHeaderList = z.infer<
  typeof defaultRequestHeaderListSchema
>;

/**
 * Update payload semantics (backend-compatible):
 * - `undefined` (omit) — leave unchanged
 * - `null` — clear all default headers
 * - `[]` — clear (backend treats empty list same as null)
 * - array — replace with validated list
 */
export const defaultRequestHeaderUpdateSchema = z
  .union([z.null(), defaultRequestHeaderListSchema])
  .optional();

export type DefaultRequestHeaderUpdate = z.infer<
  typeof defaultRequestHeaderUpdateSchema
>;
