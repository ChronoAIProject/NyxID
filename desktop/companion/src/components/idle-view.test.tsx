import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { IdleView } from "./idle-view";

function renderIdleView(
  overrides: Partial<React.ComponentProps<typeof IdleView>> = {},
) {
  const callbacks = {
    onOpen: vi.fn(),
    onMeal: vi.fn(),
    onSettings: vi.fn(),
    onStartDrag: vi.fn(() => Promise.resolve()),
    onToggleQuiet: vi.fn(),
  };
  const view = render(
    <IdleView
      name="Nyx"
      nextMeal="午饭 12:30"
      quiet={false}
      {...callbacks}
      {...overrides}
    />,
  );

  const mascotButton = screen.getByRole("button", {
    name: "摸摸 Nyx；拖动可以移动",
  });
  let capturedPointer: number | undefined;
  Object.defineProperties(mascotButton, {
    setPointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => {
        capturedPointer = pointerId;
      }),
    },
    hasPointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => capturedPointer === pointerId),
    },
    releasePointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => {
        if (capturedPointer === pointerId) capturedPointer = undefined;
      }),
    },
    getBoundingClientRect: {
      configurable: true,
      value: vi.fn(() => ({
        bottom: 190,
        height: 190,
        left: 0,
        right: 190,
        top: 0,
        width: 190,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      })),
    },
  });

  return {
    callbacks,
    mascotButton,
    rerender: (next: Partial<React.ComponentProps<typeof IdleView>>) => {
      view.rerender(
        <IdleView
          name="Nyx"
          nextMeal="午饭 12:30"
          quiet={false}
          {...callbacks}
          {...next}
        />,
      );
    },
  };
}

describe("IdleView pet interaction", () => {
  it("keeps movement below four screen pixels as a captured petting click", () => {
    const { callbacks, mascotButton } = renderIdleView();

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 7,
      screenX: 100,
      screenY: 100,
      clientX: 10,
      clientY: 10,
    });
    expect(mascotButton).toHaveAttribute("data-interaction", "pressed");
    expect(mascotButton.querySelector(".mascot--waving")).not.toBeNull();

    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 7,
      screenX: 103,
      screenY: 102,
      clientX: 210,
      clientY: 210,
    });
    fireEvent.pointerUp(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 7,
      screenX: 103,
      screenY: 102,
    });
    fireEvent.click(mascotButton);

    expect(callbacks.onStartDrag).not.toHaveBeenCalled();
    expect(callbacks.onOpen).not.toHaveBeenCalled();
    expect(mascotButton).toHaveAttribute("data-interaction", "petted");
    expect(mascotButton.querySelector(".mascot--happy")).not.toBeNull();
    expect(screen.getByText("我在呢")).toBeInTheDocument();
    expect(mascotButton.setPointerCapture).toHaveBeenCalledWith(7);
    expect(mascotButton.releasePointerCapture).toHaveBeenCalledWith(7);
  });

  it("starts a rightward drag at exactly four pixels and suppresses click", async () => {
    let finishDrag: (() => void) | undefined;
    const dragGate = new Promise<void>((resolve) => {
      finishDrag = resolve;
    });
    const onStartDrag = vi.fn(() => dragGate);
    const { callbacks, mascotButton } = renderIdleView({
      mascotState: "waiting",
      onStartDrag,
    });

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 11,
      screenX: 50,
      screenY: 60,
    });
    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 11,
      screenX: 54,
      screenY: 60,
    });

    expect(onStartDrag).toHaveBeenCalledTimes(1);
    expect(mascotButton).toHaveClass("is-dragging");
    expect(mascotButton).toHaveAttribute("data-interaction", "dragging-right");
    expect(mascotButton.querySelector(".mascot--running-right")).not.toBeNull();

    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 11,
      screenX: 70,
      screenY: 60,
    });
    expect(onStartDrag).toHaveBeenCalledTimes(1);

    fireEvent.pointerUp(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 11,
      screenX: 70,
      screenY: 60,
    });
    fireEvent.click(mascotButton);

    expect(callbacks.onOpen).not.toHaveBeenCalled();
    expect(mascotButton).not.toHaveClass("is-dragging");
    expect(mascotButton.querySelector(".mascot--waiting")).not.toBeNull();

    await act(async () => {
      finishDrag?.();
      await dragGate;
    });
  });

  it("shows leftward running, then restores the supplied service state", async () => {
    let finishDrag: (() => void) | undefined;
    const dragGate = new Promise<void>((resolve) => {
      finishDrag = resolve;
    });
    const { mascotButton } = renderIdleView({
      mascotState: "failed",
      onStartDrag: () => dragGate,
    });

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 13,
      screenX: 80,
      screenY: 80,
    });
    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 13,
      screenX: 76,
      screenY: 80,
    });
    expect(mascotButton.querySelector(".mascot--running-left")).not.toBeNull();

    fireEvent.pointerCancel(mascotButton, {
      isPrimary: true,
      pointerId: 13,
    });
    expect(mascotButton.querySelector(".mascot--failed")).not.toBeNull();

    await act(async () => {
      finishDrag?.();
      await dragGate;
    });
  });

  it("waits for the native settled event instead of the invoke promise", async () => {
    const onStartDrag = vi.fn(() => Promise.resolve());
    const { mascotButton, rerender } = renderIdleView({
      mascotState: "waiting",
      onStartDrag,
    });

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 21,
      screenX: 40,
      screenY: 40,
    });
    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 21,
      screenX: 44,
      screenY: 40,
    });
    await act(async () => Promise.resolve());

    expect(mascotButton).toHaveAttribute("data-interaction", "dragging-right");
    rerender({
      mascotState: "waiting",
      onStartDrag,
      dragEvent: { phase: "moving", direction: "left" },
    });
    expect(mascotButton).toHaveAttribute("data-interaction", "dragging-left");

    rerender({
      mascotState: "waiting",
      onStartDrag,
      dragEvent: { phase: "settled", direction: null },
    });
    expect(mascotButton).toHaveAttribute("data-interaction", "resting");
    expect(mascotButton.querySelector(".mascot--waiting")).not.toBeNull();

    fireEvent.click(mascotButton);
    expect(mascotButton).toHaveAttribute("data-interaction", "resting");
    expect(mascotButton.querySelector(".mascot--waiting")).not.toBeNull();
    expect(screen.queryByText("我在呢")).not.toBeInTheDocument();

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 22,
      screenX: 44,
      screenY: 40,
    });
    fireEvent.pointerUp(mascotButton, {
      button: 0,
      isPrimary: true,
      pointerId: 22,
      screenX: 44,
      screenY: 40,
    });
    fireEvent.click(mascotButton);

    expect(mascotButton).toHaveAttribute("data-interaction", "petted");
    expect(mascotButton.querySelector(".mascot--happy")).not.toBeNull();
  });

  it("restores the normal bubble after the short petting response", () => {
    let expireReaction: (() => void) | undefined;
    const clearTimer = vi.spyOn(window, "clearTimeout");
    const timer = vi
      .spyOn(window, "setTimeout")
      .mockImplementation((handler) => {
        if (typeof handler === "function") expireReaction = handler;
        return 1;
      });
    try {
      const { mascotButton } = renderIdleView();

      fireEvent.click(mascotButton);
      expect(screen.getByText("我在呢")).toBeInTheDocument();
      expect(clearTimer).not.toHaveBeenCalledWith(1);

      act(() => {
        expireReaction?.();
      });

      expect(screen.getByText("午饭 12:30")).toBeInTheDocument();
      expect(mascotButton).toHaveAttribute("data-interaction", "resting");
      expect(mascotButton.querySelector(".mascot--idle")).not.toBeNull();
    } finally {
      timer.mockRestore();
      clearTimer.mockRestore();
    }
  });

  it("follows the pointer on hover and ignores non-primary drag gestures", () => {
    const { callbacks, mascotButton } = renderIdleView({
      mascotState: "review",
    });

    fireEvent.pointerEnter(mascotButton, {
      isPrimary: true,
      pointerId: 17,
    });
    fireEvent.pointerMove(mascotButton, {
      isPrimary: true,
      pointerId: 17,
      clientX: 180,
      clientY: 24,
    });
    const mascot = mascotButton.querySelector<HTMLElement>(".mascot");
    expect(mascotButton.querySelector(".mascot--review")).not.toBeNull();
    expect(mascot?.style.getPropertyValue("--mascot-look-x")).toBe("0.895");
    expect(mascot?.style.getPropertyValue("--mascot-look-y")).toBe("-0.747");

    fireEvent.pointerDown(mascotButton, {
      button: 0,
      isPrimary: false,
      pointerId: 18,
      screenX: 10,
      screenY: 10,
    });
    fireEvent.pointerMove(mascotButton, {
      isPrimary: false,
      pointerId: 18,
      screenX: 80,
      screenY: 80,
    });
    expect(callbacks.onStartDrag).not.toHaveBeenCalled();

    fireEvent.pointerLeave(mascotButton, {
      isPrimary: true,
      pointerId: 17,
    });
    expect(mascotButton.querySelector(".mascot--review")).not.toBeNull();
    expect(mascot?.style.getPropertyValue("--mascot-look-x")).toBe("0.000");
    expect(mascot?.style.getPropertyValue("--mascot-look-y")).toBe("0.000");
  });

  it("keeps action controls independent from the pet drag surface", () => {
    const { callbacks } = renderIdleView();

    fireEvent.click(screen.getByRole("button", { name: "和 NyxID 对话" }));
    fireEvent.click(screen.getByRole("button", { name: "设置" }));

    expect(callbacks.onOpen).toHaveBeenCalledTimes(1);
    expect(callbacks.onSettings).toHaveBeenCalledTimes(1);
    expect(callbacks.onStartDrag).not.toHaveBeenCalled();
  });
});
