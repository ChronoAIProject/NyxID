import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n/instance";
import { responseApiError } from "@/lib/api-error";
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


it("translates a proxy ApiError at render time and keeps diagnostics and retry intact", async () => {
  const error = await responseApiError(new Response("<html>nginx</html>", {
    status: 503,
    headers: { "x-request-id": "request-2275", "X-NyxID-Trace-Id": "trace-2275" },
  }));
  const onRetry = vi.fn();
  await appI18n.changeLanguage("en");
  const view = render(<ErrorBanner message={error.message} onRetry={onRetry} />);
  try {
    expect(screen.getByText("NyxID is temporarily unavailable. Try again in a moment.")).toBeVisible();
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByText("NyxID 暂时不可用。请稍后重试。")).toBeVisible();
    expect(view.container).not.toHaveTextContent("nginx");
    expect(view.container).not.toHaveTextContent("trace-2275");
    fireEvent.click(screen.getByRole("button"));
    expect(onRetry).toHaveBeenCalledOnce();
    expect(error).toMatchObject({
      message: "NyxID is temporarily unavailable. Try again in a moment.",
      status: 503, requestId: "request-2275", traceId: "trace-2275",
    });
  } finally {
    view.unmount();
    await appI18n.changeLanguage("en");
  }
});
