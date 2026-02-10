import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  cn,
  formatDate,
  formatRelativeTime,
  maskApiKey,
  sanitizeAvatarUrl,
} from "./utils";

describe("cn", () => {
  it("merges class names", () => {
    expect(cn("foo", "bar")).toBe("foo bar");
  });

  it("handles conditional classes", () => {
    expect(cn("base", false, "visible")).toBe("base visible");
  });

  it("deduplicates tailwind classes", () => {
    expect(cn("p-4", "p-2")).toBe("p-2");
  });

  it("handles empty input", () => {
    expect(cn()).toBe("");
  });

  it("handles undefined and null values", () => {
    expect(cn("base", undefined, null, "end")).toBe("base end");
  });
});

describe("formatDate", () => {
  it("formats a valid date string", () => {
    const result = formatDate("2024-01-15T12:00:00Z");
    expect(result).toMatch(/Jan\s+15,\s+2024/);
  });

  it("returns N/A for null", () => {
    expect(formatDate(null)).toBe("N/A");
  });

  it("returns N/A for undefined", () => {
    expect(formatDate(undefined)).toBe("N/A");
  });

  it("returns N/A for empty string", () => {
    expect(formatDate("")).toBe("N/A");
  });

  it("returns N/A for invalid date string", () => {
    expect(formatDate("not-a-date")).toBe("N/A");
  });
});

describe("formatRelativeTime", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns 'just now' for times less than 60 seconds ago", () => {
    expect(formatRelativeTime("2024-06-15T11:59:30Z")).toBe("just now");
  });

  it("returns minutes ago", () => {
    expect(formatRelativeTime("2024-06-15T11:45:00Z")).toBe("15m ago");
  });

  it("returns hours ago", () => {
    expect(formatRelativeTime("2024-06-15T09:00:00Z")).toBe("3h ago");
  });

  it("returns days ago for less than 7 days", () => {
    expect(formatRelativeTime("2024-06-13T12:00:00Z")).toBe("2d ago");
  });

  it("returns formatted date for more than 7 days ago", () => {
    const result = formatRelativeTime("2024-06-01T12:00:00Z");
    expect(result).toMatch(/Jun\s+1,\s+2024/);
  });

  it("returns N/A for null", () => {
    expect(formatRelativeTime(null)).toBe("N/A");
  });

  it("returns N/A for undefined", () => {
    expect(formatRelativeTime(undefined)).toBe("N/A");
  });

  it("returns N/A for invalid date", () => {
    expect(formatRelativeTime("garbage")).toBe("N/A");
  });
});

describe("maskApiKey", () => {
  it("masks key with 24 asterisks after prefix", () => {
    expect(maskApiKey("sk-proj")).toBe("sk-proj" + "*".repeat(24));
  });

  it("handles empty prefix", () => {
    expect(maskApiKey("")).toBe("*".repeat(24));
  });
});

describe("sanitizeAvatarUrl", () => {
  it("returns https URL unchanged", () => {
    expect(sanitizeAvatarUrl("https://example.com/avatar.png")).toBe(
      "https://example.com/avatar.png",
    );
  });

  it("returns http URL unchanged", () => {
    expect(sanitizeAvatarUrl("http://example.com/avatar.png")).toBe(
      "http://example.com/avatar.png",
    );
  });

  it("returns null for null input", () => {
    expect(sanitizeAvatarUrl(null)).toBeNull();
  });

  it("returns null for undefined input", () => {
    expect(sanitizeAvatarUrl(undefined)).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(sanitizeAvatarUrl("")).toBeNull();
  });

  it("returns null for javascript: URL", () => {
    expect(sanitizeAvatarUrl("javascript:alert(1)")).toBeNull();
  });

  it("returns null for data: URL", () => {
    expect(sanitizeAvatarUrl("data:image/png;base64,abc")).toBeNull();
  });

  it("returns null for ftp: URL", () => {
    expect(sanitizeAvatarUrl("ftp://files.example.com/avatar.png")).toBeNull();
  });

  it("returns null for invalid URL", () => {
    expect(sanitizeAvatarUrl("not a url at all")).toBeNull();
  });
});
