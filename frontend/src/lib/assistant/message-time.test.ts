import { afterEach, describe, expect, it, vi } from "vitest";
import { messageTime } from "@/lib/assistant/message-time";

afterEach(() => vi.unstubAllGlobals());

/** ICU separates some parts with narrow no-break spaces. */
function label(value: Parameters<typeof messageTime>[0], timeZone: string, locale = "en-US") {
  return messageTime(value, { locale, timeZone })?.label.replace(/\s/g, " ");
}

describe("messageTime", () => {
  it("shows one instant in each reader's own time zone", () => {
    const instant = "2026-10-12T07:40:43Z";
    expect(label(instant, "America/New_York")).toBe("Oct 12, 2026, 3:40 AM EDT");
    expect(label(instant, "Asia/Tokyo")).toBe("Oct 12, 2026, 4:40 PM GMT+9");
    // The machine-readable instant never changes with the reader.
    expect(messageTime(instant, { timeZone: "America/New_York" })?.iso).toBe("2026-10-12T07:40:43.000Z");
    expect(messageTime(instant, { timeZone: "Asia/Tokyo" })?.iso).toBe("2026-10-12T07:40:43.000Z");
  });

  it("follows the reader's locale", () => {
    expect(label("2026-10-12T07:40:43Z", "UTC", "en-GB")).toBe("12 Oct 2026, 07:40 UTC");
  });

  it("prefers the browser's first requested language", () => {
    vi.stubGlobal("navigator", { languages: ["en-GB"], language: "en-US" });
    expect(messageTime("2026-10-12T07:40:43Z", { timeZone: "UTC" })?.label)
      .toBe("12 Oct 2026, 07:40 UTC");
  });

  it.each([undefined, []])("uses the browser's language when its language list is %s", (languages) => {
    vi.stubGlobal("navigator", { languages, language: "en-GB" });
    expect(messageTime("2026-10-12T07:40:43Z", { timeZone: "UTC" })?.label)
      .toBe("12 Oct 2026, 07:40 UTC");
  });

  it.each([undefined, {}])("formats timestamps when browser locale preferences are unavailable: %s", (browser) => {
    vi.stubGlobal("navigator", browser);
    const labels = messageTime("2026-10-12T07:40:43Z", { timeZone: "UTC" });
    expect(labels?.iso).toBe("2026-10-12T07:40:43.000Z");
    expect(labels?.label).toBeTruthy();
  });

  it("places instants on the reader's local calendar day", () => {
    const lateUtc = "2026-10-12T23:30:00Z";
    const earlyUtc = "2026-10-13T00:30:00Z";
    expect(label(lateUtc, "UTC")).toBe("Oct 12, 2026, 11:30 PM UTC");
    expect(label(earlyUtc, "UTC")).toBe("Oct 13, 2026, 12:30 AM UTC");
    // East of UTC both fall on the 13th; west of it both fall on the 12th.
    expect(label(lateUtc, "Asia/Tokyo")).toBe("Oct 13, 2026, 8:30 AM GMT+9");
    expect(label(earlyUtc, "Asia/Tokyo")).toBe("Oct 13, 2026, 9:30 AM GMT+9");
    expect(label(lateUtc, "America/Los_Angeles")).toBe("Oct 12, 2026, 4:30 PM PDT");
    expect(label(earlyUtc, "America/Los_Angeles")).toBe("Oct 12, 2026, 5:30 PM PDT");
    // New Year's Eve in UTC is already New Year's Day in Tokyo.
    expect(label("2026-12-31T16:00:00Z", "Asia/Tokyo")).toBe("Jan 1, 2027, 1:00 AM GMT+9");
  });

  it("keeps the repeated hour of a daylight-saving change apart", () => {
    expect(label("2026-11-01T05:30:00Z", "America/New_York")).toBe("Nov 1, 2026, 1:30 AM EDT");
    expect(label("2026-11-01T06:30:00Z", "America/New_York")).toBe("Nov 1, 2026, 1:30 AM EST");
  });

  it("accepts epoch milliseconds", () => {
    expect(label(Date.parse("2026-10-12T07:40:43Z"), "UTC")).toBe("Oct 12, 2026, 7:40 AM UTC");
  });

  describe("short label", () => {
    const now = Date.parse("2026-10-28T04:00:00Z");
    function short(value: string, timeZone: string, locale = "en-US") {
      return messageTime(value, { locale, timeZone, now })?.short.replace(/\s/g, " ");
    }

    it("shows only the day and month within the current year, day first for English", () => {
      expect(short("2026-10-27T00:04:00Z", "Asia/Singapore")).toBe("27 Oct");
      expect(short("2026-01-02T12:00:00Z", "UTC", "en-GB")).toBe("2 Jan");
    });

    it("adds the year outside the current one", () => {
      expect(short("2025-10-27T00:04:00Z", "Asia/Singapore")).toBe("27 Oct 2025");
      expect(short("2027-01-02T12:00:00Z", "UTC")).toBe("2 Jan 2027");
    });

    it("judges the year on the reader's own calendar", () => {
      // New Year's Eve in UTC is already next year in Tokyo.
      expect(short("2026-12-31T16:00:00Z", "UTC")).toBe("31 Dec");
      expect(short("2026-12-31T16:00:00Z", "Asia/Tokyo")).toBe("1 Jan 2027");
    });

    it("keeps other languages' own day and month order", () => {
      expect(short("2026-10-27T00:04:00Z", "Asia/Shanghai", "zh-CN")).toBe("10月27日");
    });
  });

  it("yields nothing for a missing or unparseable instant", () => {
    for (const value of [undefined, null, "", "not a date", Number.NaN, Date.parse("garbage")]) {
      expect(messageTime(value)).toBeUndefined();
    }
  });
});
