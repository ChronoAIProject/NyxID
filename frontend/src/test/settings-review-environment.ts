import { expect, vi } from "vitest";
import { act, waitFor } from "@testing-library/react";
import type { QueryClient } from "@tanstack/react-query";
import { appI18n } from "@/i18n/instance";

/** Base captures and current renders use the same real Intl formatting and clock. */
export async function settingsReviewEnvironment() {
  await appI18n.changeLanguage("en");
  vi.setSystemTime(new Date("2026-10-15T12:00:00Z"));
  // Follow billing-bill.test.tsx: delegate to native formatting, with explicit
  // locale/timezone rather than replacing date text or normalizing the figures.
  for (const method of ["toLocaleString", "toLocaleDateString", "toLocaleTimeString"] as const) {
    const original = Date.prototype[method];
    vi.spyOn(Date.prototype, method).mockImplementation(function (this: Date, _locale, options) {
      return original.call(this, "en-US", { ...options, timeZone: options?.timeZone ?? "UTC" });
    });
  }
  const DateTimeFormat = Intl.DateTimeFormat;
  vi.spyOn(Intl, "DateTimeFormat").mockImplementation(function (_locale, options) {
    return new DateTimeFormat("en-US", { ...options, timeZone: options?.timeZone ?? "UTC" });
  });
}

/** Await the fixture queries and React commits instead of sleeping per page. */
export async function settleSettingsReviewQueries(client: QueryClient) {
  await waitFor(() => expect(client.isFetching()).toBe(0), { timeout: 5_000, interval: 10 });
  await act(async () => {});
}
