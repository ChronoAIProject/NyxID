import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ErrorBanner } from "./error-banner";

describe("ErrorBanner retry lifecycle", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    cleanup();
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it("deduplicates retries for 1500ms and permits another retry afterward", () => {
    const onRetry = vi.fn();
    render(<ErrorBanner message="Unavailable" onRetry={onRetry} />);
    const button = screen.getByRole("button", { name: "Retry" });

    fireEvent.click(button);
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(onRetry).toHaveBeenCalledTimes(1);

    act(() => vi.advanceTimersByTime(1499));
    expect(button).toBeDisabled();
    act(() => vi.advanceTimersByTime(1));
    expect(button).toBeEnabled();
    fireEvent.click(button);
    expect(onRetry).toHaveBeenCalledTimes(2);
    expect(button).toBeDisabled();
  });

  it("cancels the pending retry cooldown when unmounted", () => {
    const view = render(<ErrorBanner message="Unavailable" onRetry={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(vi.getTimerCount()).toBe(1);

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    act(() => vi.advanceTimersByTime(1500));
    expect(vi.getTimerCount()).toBe(0);
  });

  it("leaves no cooldown when retry immediately removes the banner", () => {
    function RetryView() {
      const [visible, setVisible] = useState(true);
      return visible ? (
        <ErrorBanner message="Unavailable" onRetry={() => setVisible(false)} />
      ) : null;
    }

    render(<RetryView />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(screen.queryByText("Unavailable")).not.toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
  });
});
