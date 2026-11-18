import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render as baseRender, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { OfflineBanner } from "./offline-banner";

function render(ui: ReactElement) {
  return baseRender(<QueryClientProvider client={new QueryClient()}>{ui}</QueryClientProvider>);
}

let online = true;
const original = Object.getOwnPropertyDescriptor(Navigator.prototype, "onLine");
function setOnline(next: boolean) {
  online = next;
  act(() => {
    window.dispatchEvent(new Event(next ? "online" : "offline"));
  });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  online = true;
  if (original) Object.defineProperty(Navigator.prototype, "onLine", original);
});

it("says so across the top while offline, then briefly that it is back", () => {
  vi.useFakeTimers();
  Object.defineProperty(Navigator.prototype, "onLine", { configurable: true, get: () => online });
  render(<OfflineBanner />);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  setOnline(false);
  expect(screen.getByRole("alert")).toHaveTextContent(
    "You're offline. NyxBot can't send, run or refresh until you reconnect. You're seeing what was already loaded.",
  );
  setOnline(true);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Back online.");
  act(() => {
    vi.advanceTimersByTime(3000);
  });
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("says NyxID is unreachable when a request fails though the browser is online, until one succeeds", async () => {
  const { markConnectionLost, markConnectionOk } = await import("@/lib/assistant/connection-state");
  render(<OfflineBanner />);
  act(() => markConnectionLost());
  expect(screen.getByRole("alert")).toHaveTextContent("Can't reach NyxID.");
  act(() => markConnectionOk());
  expect(screen.getByRole("status")).toHaveTextContent("Back online.");
});
