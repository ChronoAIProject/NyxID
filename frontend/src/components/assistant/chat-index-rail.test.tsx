import { createRef } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ChatIndexRail } from "./chat-index-rail";
import type { ChatMessage } from "@/lib/assistant/chat-types";

const messages: ChatMessage[] = Array.from({ length: 5000 }, (_, index) => ({ id: `message-${index}`, role: index % 2 ? "assistant" : "user", content: `Message ${index}`, timestamp: 0, status: "complete" }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function geometry(height = 80) {
  const resizes: (() => void)[] = [];
  const state = { pitch: 7, bottom: 70 };
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resizes.push(callback); } observe() {} unobserve() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    let top = 20;
    if (this.hasAttribute("data-chat-index-rail") || this.hasAttribute("data-test-transcript")) top = 100;
    if (this.hasAttribute("data-chat-message-index")) top = 100 + Number(this.dataset.chatMessageIndex) * 100 - this.parentElement!.scrollTop;
    const elementHeight = this.hasAttribute("data-chat-index-pitch") ? state.pitch : height;
    return { top, bottom: this.hasAttribute("data-chat-header-controls") ? state.bottom : top + elementHeight, height: elementHeight } as DOMRect;
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(height);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
    if (this.hasAttribute("data-chat-index-rail")) return Math.max(height, parseFloat((this.firstElementChild as HTMLElement).style.height));
    return this.dataset.atEnd === "true" ? height : 1_000_000;
  });
  return { state, resizes };
}
function harness(count: number, atEnd = false) {
  const scroll = createRef<HTMLDivElement>();
  const navigate = vi.fn();
  const view = render(<div data-chat-content-layout=""><div data-chat-header-controls="" /><div>
    <div ref={scroll} data-test-transcript="" data-at-end={atEnd}>{Array.from({ length: Math.min(count, 100) }, (_, index) => <div key={index} data-chat-message-index={index} />)}</div>
    <ChatIndexRail messages={messages.slice(0, count)} scrollRef={scroll} bottomInset={90} onNavigate={navigate} />
  </div></div>);
  return { ...view, scroll, navigate, nav: screen.getByRole("navigation", { name: "Conversation index" }) };
}

it("uses dense text-scaled slots below the toolbar and updates when the toolbar wraps without writing during observer delivery", async () => {
  const { state, resizes } = geometry(450);
  const { nav } = harness(80);
  expect(nav.parentElement).not.toHaveClass("bg-sidebar", "border-l");
  expect(nav).toHaveClass("overflow-y-auto", "scrollbar-none");
  expect(nav.parentElement).toHaveStyle({ top: "80px" });
  expect(nav.querySelectorAll("button")).toHaveLength(73);
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "7px" });
  state.pitch = 10.5; state.bottom = 110;
  act(() => resizes[0]!());
  expect(nav.parentElement).toHaveStyle({ top: "80px" });
  await waitFor(() => expect(nav.parentElement).toHaveStyle({ top: "98px" }));
  await waitFor(() => expect(nav.querySelectorAll("button")).toHaveLength(51));
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "10.5px" });
});

it("coalesces resize notifications into one frame and cancels pending measurements on unmount", () => {
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  const request = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => { frames.delete(id); });
  const { state, resizes } = geometry();
  const { nav, unmount } = harness(100);
  request.mockClear();
  state.bottom = 110;
  act(() => { resizes[0]!(); resizes[0]!(); });
  expect(request).toHaveBeenCalledTimes(1);
  expect(nav.parentElement).toHaveStyle({ top: "80px" });
  const scheduled = frameId;
  act(() => { frames.get(scheduled)!(0); frames.delete(scheduled); });
  expect(nav.parentElement).toHaveStyle({ top: "98px" });
  act(() => resizes[0]!());
  const pending = frameId;
  unmount();
  expect(cancel).toHaveBeenCalledWith(pending);
  expect(frames.has(pending)).toBe(false);
});

it.each([4, 5])("keeps only the current tick violet, including either role, with neutral previews and jumps (%i messages)", async (count) => {
  geometry();
  const { nav, navigate } = harness(count, true);
  await waitFor(() => expect(nav.querySelector('[aria-current="location"]')).not.toBeNull());
  expect(nav.querySelector('[aria-current="location"]')).toHaveAttribute("data-message-role", count % 2 ? "user" : "assistant");
  expect(nav.querySelector('[aria-current="location"] > span')).toHaveClass("text-nyx-secondary-400", "opacity-100");
  expect(nav.querySelector('[aria-current="location"] > span')).toHaveClass(count % 2 ? "w-[calc(var(--chat-index-tick-width)/2)]" : "w-[var(--chat-index-tick-width)]");
  expect(nav.querySelector('[data-message-role="user"]:not([aria-current]) > span')).toHaveClass("w-[calc(var(--chat-index-tick-width)/2)]");
  expect(nav.querySelector('[data-message-role="assistant"] > span')).toHaveClass("w-[var(--chat-index-tick-width)]");
  for (const tick of nav.querySelectorAll("button > span")) expect(tick).toHaveClass("h-[var(--chat-index-tick-thickness)]", "rounded-full", "bg-current");
  expect(nav.querySelector('button:not([aria-current]) > span')).toHaveClass("text-text-tertiary", "opacity-50", "group-hover:text-muted-foreground", "group-hover:opacity-100", "group-focus-visible:text-muted-foreground", "group-focus-visible:opacity-100", "group-active:text-muted-foreground", "group-active:opacity-100");
  const first = nav.querySelector("button")!;
  expect(first).toHaveClass("focus-visible:ring-muted-foreground");
  fireEvent.pointerEnter(first, { pointerType: "mouse" });
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  nav.setPointerCapture = vi.fn();
  fireEvent.pointerDown(first, { button: 0, pointerId: 1, clientY: 100 });
  expect(screen.getByRole("tooltip")).toHaveTextContent("Message 0");
  fireEvent.pointerUp(nav, { pointerId: 1 });
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  expect(first.firstElementChild).not.toHaveClass("text-nyx-secondary-400");
  fireEvent.click(first, { detail: 0 });
  expect(navigate).toHaveBeenLastCalledWith(0);
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(navigate).toHaveBeenLastCalledWith(1);
  expect(nav.querySelector('[data-chat-index-target="1"]')).toHaveFocus();
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", ""));
});

it("scrolls very long rails with bounded rendering and fades only edges hiding ticks", async () => {
  geometry();
  const { nav } = harness(5000);
  expect(nav.firstElementChild).toHaveStyle({ height: "35000px" });
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", "bottom"));
  expect(nav.querySelectorAll("button").length).toBeLessThan(30);
  nav.scrollTop = 21000;
  fireEvent.scroll(nav);
  expect(nav).toHaveAttribute("data-scroll-edges", "top bottom");
  expect(nav.querySelector('[data-chat-index-target="3000"]')).not.toBeNull();
  expect(nav.querySelectorAll("button").length).toBeLessThan(30);
  nav.scrollTop = 34920;
  fireEvent.scroll(nav);
  expect(nav).toHaveAttribute("data-scroll-edges", "top");
  nav.scrollTop = 0;
  fireEvent.scroll(nav);
  expect(nav).toHaveAttribute("data-scroll-edges", "bottom");
});

it("reveals and focuses keyboard destinations outside the rendered window", async () => {
  geometry();
  const { nav, navigate } = harness(5000);
  fireEvent.keyDown(nav.querySelector("button")!, { key: "End" });
  fireEvent.scroll(nav);
  expect(nav.scrollTop).toBe(34920);
  expect(navigate).toHaveBeenLastCalledWith(4999);
  expect(nav.querySelector('[data-chat-index-target="4999"]')).toHaveFocus();
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", "top"));
  fireEvent.keyDown(document.activeElement!, { key: "Home" });
  fireEvent.scroll(nav);
  expect(nav.scrollTop).toBe(0);
  expect(navigate).toHaveBeenLastCalledWith(0);
  expect(nav.querySelector('[data-chat-index-target="0"]')).toHaveFocus();
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", "bottom"));
  fireEvent.keyDown(document.activeElement!, { key: "PageDown" });
  expect(navigate).toHaveBeenLastCalledWith(10);
  expect(nav.querySelector('[data-chat-index-target="10"]')).toHaveFocus();
});

it("follows transcript changes without fading the newly active tick", async () => {
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => { frames.delete(id); });
  async function nextFrame() {
    // Callbacks queued by this frame run in the next one. Async act also
    // delivers the DOM mutations that schedule the coalesced fade refresh.
    const pending = [...frames.values()];
    frames.clear();
    await act(async () => { pending.forEach((callback) => callback(0)); });
  }
  geometry();
  const { nav, scroll } = harness(100);
  await nextFrame();
  expect(nav).toHaveAttribute("data-scroll-edges", "bottom");
  scroll.current!.scrollTop = 5000;
  fireEvent.scroll(scroll.current!);
  await nextFrame();
  expect(nav.querySelector('[data-chat-index-target="50"]')).toHaveAttribute("aria-current", "location");
  expect(nav.scrollTop).toBe(297);
  // Browsers emit scroll after reveal writes scrollTop; happy-dom does not.
  // Deliver it explicitly instead of relying on incidental mutation delivery.
  fireEvent.scroll(nav);
  await nextFrame();
  expect(nav).toHaveAttribute("data-scroll-edges", "top bottom");
});

it("keeps pointer selection on the dense tick slots during a drag", () => {
  geometry();
  const { nav, navigate } = harness(100);
  nav.setPointerCapture = vi.fn();
  fireEvent.pointerDown(nav.querySelector('[data-chat-index-target="4"]')!, { pointerId: 1, button: 0, clientY: 131.5 });
  fireEvent.pointerMove(nav, { pointerId: 1, clientY: 159.5 });
  expect(navigate).toHaveBeenLastCalledWith(8);
  fireEvent.pointerUp(nav, { pointerId: 1 });
});

it("mounts the scroll behavior when a new thread grows from one tick to two", async () => {
  geometry();
  const scroll = createRef<HTMLDivElement>();
  const ui = (count: number) => <div><div ref={scroll} data-at-end="true" /><ChatIndexRail messages={messages.slice(0, count)} scrollRef={scroll} bottomInset={90} onNavigate={vi.fn()} /></div>;
  const { rerender } = render(ui(1));
  expect(screen.queryByRole("navigation")).toBeNull();
  rerender(ui(2));
  const nav = screen.getByRole("navigation", { name: "Conversation index" });
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", ""));
  await waitFor(() => expect(nav.querySelector('[data-chat-index-target="1"]')).toHaveAttribute("aria-current", "location"));
});

it("anchors to the transcript column and fills its visible height without subtracting the composer twice", async () => {
  const { resizes } = geometry(600);
  let right = 1000;
  let composerTop = 700;
  const rect = (top: number, bottom: number, edge = right) => ({ top, bottom, left: 250, right: edge, width: edge - 250, height: bottom - top }) as DOMRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this.hasAttribute("data-chat-index-pitch")) return rect(0, 7);
    if (this.hasAttribute("data-chat-header-controls")) return rect(20, 85);
    if (this.hasAttribute("data-composer-dock")) return rect(composerTop, 800);
    return rect(100, 800);
  });
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(750);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(735);
  const scroll = createRef<HTMLDivElement>();
  const { container } = render(<div data-chat-content-layout="">
    <div data-chat-header-controls="" />
    <div data-chat-transcript-viewport="" className="relative">
      <div ref={scroll} data-chat-transcript="" />
      <ChatIndexRail messages={messages.slice(0, 200)} scrollRef={scroll} bottomInset={200} onNavigate={vi.fn()} />
    </div>
    <div data-composer-dock="" />
    <aside aria-label="Preview" />
  </div>);
  const railContainer = container.querySelector("[data-chat-index-container]")!;
  expect(railContainer.parentElement).toHaveAttribute("data-chat-transcript-viewport");
  expect(railContainer).toHaveStyle({ top: "0px", bottom: "100px", right: "15px" });
  expect(railContainer).toHaveClass("absolute", "inset-y-0", "hidden", "md:flex");
  // Resizing the adjacent column and composer remeasures in a single frame.
  right = 620; composerTop = 640;
  act(() => resizes[0]!());
  expect(railContainer).toHaveStyle({ bottom: "100px" });
  await waitFor(() => expect(railContainer).toHaveStyle({ top: "0px", bottom: "160px", right: "15px" }));
});
