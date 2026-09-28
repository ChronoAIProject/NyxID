import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useServiceCardTransition } from "./use-service-card-transition";

const originalTransition = Object.getOwnPropertyDescriptor(
  document,
  "startViewTransition",
);
let card: HTMLElement;
let scroller: HTMLElement | undefined;
let scroll = vi.fn<HTMLElement["scrollIntoView"]>();

function installTransition() {
  let finish!: () => void;
  const finished = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const transition = { finished, skipTransition: vi.fn() };
  const start = vi.fn((update: () => void) => {
    update();
    return transition;
  });
  Object.defineProperty(document, "startViewTransition", {
    configurable: true,
    value: start,
  });
  return { finish, transition, start };
}

beforeEach(() => {
  card = document.createElement("section");
  scroll = vi.fn();
  card.scrollIntoView = scroll;
  document.body.append(card);
});
afterEach(() => {
  cleanup();
  card.remove();
  scroller?.remove();
  scroller = undefined;
  vi.restoreAllMocks();
  if (originalTransition)
    Object.defineProperty(document, "startViewTransition", originalTransition);
  else Reflect.deleteProperty(document, "startViewTransition");
});

describe("service card scrolling", () => {
  it("leaves space below the live toolbar height and the padded scroll viewport", async () => {
    const { finish } = installTransition();
    scroller = document.createElement("main");
    scroller.style.paddingTop = "16px";
    scroller.scrollTop = 40;
    Object.defineProperty(scroller, "clientTop", { value: 2 });
    scroller.getBoundingClientRect = () => new DOMRect(0, 100, 800, 900);
    scroller.scrollTo = vi.fn();
    const toolbar = document.createElement("div");
    toolbar.style.top = "0px";
    toolbar.getBoundingClientRect = () => new DOMRect(0, 116, 800, 200);
    card.getBoundingClientRect = () => new DOMRect(0, 650, 800, 300);
    document.body.append(scroller);
    scroller.append(toolbar, card);
    const { result } = renderHook(useServiceCardTransition);
    act(() => result.current(() => {}, card, toolbar));
    expect(scroller.scrollTo).not.toHaveBeenCalled();
    toolbar.getBoundingClientRect = () => new DOMRect(0, 116, 800, 280);
    await act(async () => {
      finish();
    });
    expect(scroller.scrollTo).toHaveBeenCalledWith({
      top: 260,
      behavior: "smooth",
    });
    expect(scroll).not.toHaveBeenCalled();
  });

  it("waits until the card transition finishes before scrolling", async () => {
    const { finish } = installTransition();
    const { result } = renderHook(useServiceCardTransition);
    const update = vi.fn();
    act(() => result.current(update, card));
    expect(update).toHaveBeenCalledOnce();
    expect(scroll).not.toHaveBeenCalled();
    await act(async () => {
      finish();
    });
    expect(scroll).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "start",
      inline: "nearest",
    });
  });

  it("cancels a pending scroll when the card is collapsed during expansion", async () => {
    const first = installTransition();
    const { result } = renderHook(useServiceCardTransition);
    act(() => result.current(() => {}, card));
    const second = installTransition();
    act(() => result.current(() => {}));
    expect(first.transition.skipTransition).toHaveBeenCalledOnce();
    await act(async () => {
      first.finish();
      second.finish();
    });
    expect(scroll).not.toHaveBeenCalled();
  });

  it("scrolls immediately without animation when reduced motion is preferred", () => {
    const { start } = installTransition();
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
    } as MediaQueryList);
    const { result } = renderHook(useServiceCardTransition);
    act(() => result.current(() => {}, card));
    expect(start).not.toHaveBeenCalled();
    expect(scroll).toHaveBeenCalledWith({
      behavior: "instant",
      block: "start",
      inline: "nearest",
    });
  });

  it("does not scroll after leaving the service view", async () => {
    const { finish, transition } = installTransition();
    const { result, unmount } = renderHook(useServiceCardTransition);
    act(() => result.current(() => {}, card));
    unmount();
    await act(async () => {
      finish();
    });
    expect(transition.skipTransition).toHaveBeenCalledOnce();
    expect(scroll).not.toHaveBeenCalled();
    expect(document.documentElement).not.toHaveClass("service-card-transition");
  });
});
