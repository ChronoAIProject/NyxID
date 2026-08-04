import { describe, it, expect } from "vitest";
import {
  defaultRequestHeaderSchema,
  defaultRequestHeaderListSchema,
  defaultRequestHeaderUpdateSchema,
  isDenylistedHeaderName,
  MAX_DEFAULT_HEADERS,
  MAX_HEADER_VALUE_LEN,
  REDACTED_PLACEHOLDER,
} from "./default-request-headers";

function row(
  name: string,
  value: string,
  overrides: Partial<{ overridable: boolean; sensitive: boolean }> = {},
) {
  return {
    name,
    value,
    overridable: overrides.overridable ?? false,
    sensitive: overrides.sensitive ?? false,
  };
}

describe("isDenylistedHeaderName", () => {
  it("rejects exact-match reserved names (case-insensitive)", () => {
    for (const n of [
      "Authorization",
      "authorization",
      "COOKIE",
      "Set-Cookie",
      "Host",
      "Content-Length",
      "User-Agent",
      "connection",
      "keep-alive",
      "x-real-ip",
      // RFC 7239 Forwarded: trust-boundary header, must stay blocked
      // in lock-step with the backend.
      "Forwarded",
      "forwarded",
      "FORWARDED",
    ]) {
      expect(isDenylistedHeaderName(n)).toBe(true);
    }
  });

  it("rejects reserved prefixes", () => {
    for (const n of [
      "x-nyxid-internal",
      "X-NyxID-Delegation-Token",
      "x-forwarded-for",
      "X-Forwarded-Proto",
      "proxy-authorization",
      "Proxy-Connection",
      // sec-websocket-* protocol-managed headers (mirrors backend).
      "sec-websocket-key",
      "Sec-WebSocket-Version",
      "Sec-WebSocket-Extensions",
      "Sec-WebSocket-Protocol",
    ]) {
      expect(isDenylistedHeaderName(n)).toBe(true);
    }
  });

  it("accepts non-reserved names", () => {
    for (const n of [
      "x-openclaw-scopes",
      "X-Api-Version",
      "X-Custom-Tenant",
      "X-Requested-With",
    ]) {
      expect(isDenylistedHeaderName(n)).toBe(false);
    }
  });
});

describe("defaultRequestHeaderSchema", () => {
  it("accepts a minimal valid entry", () => {
    const result = defaultRequestHeaderSchema.safeParse(
      row("X-Api-Version", "v2"),
    );
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.overridable).toBe(false);
      expect(result.data.sensitive).toBe(false);
    }
  });

  it("accepts names / values with surrounding whitespace (backend trims on store)", () => {
    // The schema deliberately does not transform inputs — see the note in
    // default-request-headers.ts. Whitespace is kept at validation time and
    // the backend normalizes it on write.
    const result = defaultRequestHeaderSchema.safeParse(
      row("  X-Api-Version  ", "  v2  "),
    );
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.name).toBe("  X-Api-Version  ");
      expect(result.data.value).toBe("  v2  ");
    }
  });

  it("accepts overridable + sensitive flags", () => {
    const result = defaultRequestHeaderSchema.safeParse(
      row("X-Tenant", "acme", { overridable: true, sensitive: true }),
    );
    expect(result.success).toBe(true);
  });

  it("rejects empty name", () => {
    const result = defaultRequestHeaderSchema.safeParse(row("   ", "v"));
    expect(result.success).toBe(false);
  });

  it("rejects invalid token characters in name", () => {
    for (const bad of ["bad header", "bad:header", "bad/header", "bad(h)"]) {
      const result = defaultRequestHeaderSchema.safeParse(row(bad, "v"));
      expect(result.success).toBe(false);
    }
  });

  it("rejects denylisted names with explanatory message", () => {
    for (const bad of [
      "Authorization",
      "Cookie",
      "Host",
      "User-Agent",
      "x-nyxid-internal",
      "x-forwarded-for",
      "proxy-authorization",
    ]) {
      const result = defaultRequestHeaderSchema.safeParse(row(bad, "v"));
      expect(result.success).toBe(false);
      if (!result.success) {
        const issue = result.error.issues.find((i) => i.path[0] === "name");
        expect(issue).toBeDefined();
      }
    }
  });

  it("rejects CR/LF/NUL in value", () => {
    for (const bad of [
      "line1\r\nX-Inject: yes",
      "one\ntwo",
      "with\0nul",
    ]) {
      const result = defaultRequestHeaderSchema.safeParse(row("x-test", bad));
      expect(result.success).toBe(false);
    }
  });

  it("rejects other control characters in value", () => {
    const result = defaultRequestHeaderSchema.safeParse(
      row("x-test", "with\x07bell"),
    );
    expect(result.success).toBe(false);
  });

  it("accepts tab inside value", () => {
    const result = defaultRequestHeaderSchema.safeParse(
      row("x-test", "col1\tcol2"),
    );
    expect(result.success).toBe(true);
  });

  it("rejects DEL and non-ASCII bytes (matches backend HeaderValue rules)", () => {
    // The backend `is_valid_header_value` (and the transport
    // `HeaderValue::from_str`) reject anything outside HTAB + visible
    // ASCII. The schema must catch those values in the editor — otherwise
    // the form submits cleanly and every proxy call for the service
    // fails at send time.
    for (const bad of [
      "café",          // non-ASCII latin-1
      "привет",        // non-ASCII Cyrillic
      "hi ☃",          // non-ASCII emoji/symbol
      "bad\x7Fbad",    // DEL
      "ctrl\x11byte",  // arbitrary C0 control
    ]) {
      const result = defaultRequestHeaderSchema.safeParse(row("x-test", bad));
      expect(result.success, `expected ${JSON.stringify(bad)} to be rejected`).toBe(false);
    }
  });

  it("accepts the redaction placeholder on sensitive rows (GET→edit→PUT round trip)", () => {
    // The backend returns `•••••` in place of stored sensitive values.
    // If the editor resubmits the row unchanged on an unrelated edit,
    // the schema must accept the placeholder — otherwise users would
    // have to retype every sensitive header on every save. The server's
    // `reconcile_with_stored` swaps the placeholder back to the real
    // value before persisting.
    const result = defaultRequestHeaderSchema.safeParse(
      row("X-Gateway-Token", REDACTED_PLACEHOLDER, { sensitive: true }),
    );
    expect(result.success).toBe(true);
  });

  it("still rejects the placeholder on non-sensitive rows", () => {
    // Non-sensitive values are never redacted, so a literal bullet
    // sequence there is just a non-ASCII string and should be caught.
    const result = defaultRequestHeaderSchema.safeParse(
      row("X-Public", REDACTED_PLACEHOLDER, { sensitive: false }),
    );
    expect(result.success).toBe(false);
  });

  it("accepts common valid ASCII values", () => {
    for (const ok of [
      "v2",
      "operator.read,operator.write",
      "Bearer 123",
      "a=1; b=2",
      "col\tsep",
    ]) {
      const result = defaultRequestHeaderSchema.safeParse(row("x-test", ok));
      expect(result.success, `expected ${JSON.stringify(ok)} to pass`).toBe(true);
    }
  });

  it("rejects value longer than cap", () => {
    const tooLong = "v".repeat(MAX_HEADER_VALUE_LEN + 1);
    const result = defaultRequestHeaderSchema.safeParse(row("x-test", tooLong));
    expect(result.success).toBe(false);
  });
});

describe("defaultRequestHeaderListSchema", () => {
  it("accepts an empty list", () => {
    const result = defaultRequestHeaderListSchema.safeParse([]);
    expect(result.success).toBe(true);
  });

  it("rejects more than the cap", () => {
    const many = Array.from({ length: MAX_DEFAULT_HEADERS + 1 }, (_, i) =>
      row(`x-h-${String(i)}`, "v"),
    );
    const result = defaultRequestHeaderListSchema.safeParse(many);
    expect(result.success).toBe(false);
  });

  it("rejects duplicate names (case-insensitive)", () => {
    const result = defaultRequestHeaderListSchema.safeParse([
      row("X-Api-Version", "v1"),
      row("x-api-version", "v2"),
    ]);
    expect(result.success).toBe(false);
    if (!result.success) {
      const dup = result.error.issues.find((i) =>
        String(i.message).toLowerCase().includes("duplicate"),
      );
      expect(dup).toBeDefined();
    }
  });

  it("accepts multiple distinct headers at the cap", () => {
    const items = Array.from({ length: MAX_DEFAULT_HEADERS }, (_, i) =>
      row(`x-h-${String(i)}`, "v"),
    );
    const result = defaultRequestHeaderListSchema.safeParse(items);
    expect(result.success).toBe(true);
  });
});

describe("defaultRequestHeaderUpdateSchema", () => {
  it("accepts null (clear)", () => {
    const result = defaultRequestHeaderUpdateSchema.safeParse(null);
    expect(result.success).toBe(true);
  });

  it("accepts undefined (omit = leave unchanged)", () => {
    const result = defaultRequestHeaderUpdateSchema.safeParse(undefined);
    expect(result.success).toBe(true);
  });

  it("accepts a valid array (replace)", () => {
    const result = defaultRequestHeaderUpdateSchema.safeParse([
      row("x-api-version", "v2"),
    ]);
    expect(result.success).toBe(true);
  });

  it("rejects invalid entries inside the array", () => {
    const result = defaultRequestHeaderUpdateSchema.safeParse([
      row("Authorization", "Bearer secret"),
    ]);
    expect(result.success).toBe(false);
  });
});
