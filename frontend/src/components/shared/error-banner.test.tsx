import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ErrorBanner } from "./error-banner";

afterEach(() => { cleanup(); vi.useRealTimers(); });

it("keeps retry disabled during its cooldown and releases the timer on unmount", () => {
  vi.useFakeTimers();
  const onRetry = vi.fn();
  const { unmount } = render(<ErrorBanner message="Could not load usage" onRetry={onRetry} />);
  const retry = screen.getByRole("button", { name: "Retry" });
  fireEvent.click(retry);
  expect(onRetry).toHaveBeenCalledTimes(1);
  expect(retry).toBeDisabled();
  act(() => vi.advanceTimersByTime(1499));
  expect(retry).toBeDisabled();
  act(() => vi.advanceTimersByTime(1));
  expect(retry).toBeEnabled();
  fireEvent.click(retry);
  expect(onRetry).toHaveBeenCalledTimes(2);
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
  act(() => vi.runAllTimers());
});
