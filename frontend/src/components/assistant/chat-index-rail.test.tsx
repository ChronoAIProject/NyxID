import { createRef } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ChatIndexRail } from "./chat-index-rail";
import type { ChatMessage } from "@/lib/assistant/chat-types";

const messages: ChatMessage[] = Array.from({ length: 5000 }, (_, index) => ({ id: `message-${index}`, role: index % 2 ? "assistant" : "user", content: `Message ${index}`, timestamp: 0, status: "complete" }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function geometry(height = 80) {
  const resizes: (() => void)[] = [];
  const state = { pitch: 8, bottom: 70 };
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

it("uses dense text-scaled slots below the toolbar and updates when the toolbar wraps", () => {
  const { state, resizes } = geometry(450);
  const { nav } = harness(80);
  expect(nav.parentElement).not.toHaveClass("bg-sidebar", "border-l");
  expect(nav).toHaveClass("overflow-y-auto", "scrollbar-none");
  expect(nav.parentElement).toHaveStyle({ top: "58px" });
  expect(nav.querySelectorAll("button")).toHaveLength(65);
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "8px" });
  state.pitch = 12; state.bottom = 110;
  act(() => resizes[0]!());
  expect(nav.parentElement).toHaveStyle({ top: "98px" });
  expect(nav.querySelectorAll("button")).toHaveLength(46);
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "12px" });
});

it.each([4, 5])("keeps only the current tick violet, including either role, with neutral previews and jumps (%i messages)", async (count) => {
  geometry();
  const { nav, navigate } = harness(count, true);
  await waitFor(() => expect(nav.querySelector('[aria-current="location"]')).not.toBeNull());
  expect(nav.querySelector('[aria-current="location"]')).toHaveAttribute("data-message-role", count % 2 ? "user" : "assistant");
  expect(nav.querySelector('[aria-current="location"] > span')).toHaveClass("text-nyx-secondary-400", "w-[var(--chat-index-tick-width)]");
  expect(nav.querySelector('[data-message-role="user"]:not([aria-current]) > span')).toHaveClass("w-[calc(var(--chat-index-tick-width)/2)]");
  expect(nav.querySelector('[data-message-role="assistant"] > span')).toHaveClass("w-[var(--chat-index-tick-width)]");
  for (const tick of nav.querySelectorAll("button > span")) expect(tick).toHaveClass("h-[var(--chat-index-tick-thickness)]", "rounded-full", "bg-current");
  expect(nav.querySelector('button:not([aria-current]) > span')).toHaveClass("text-text-tertiary", "group-hover:text-muted-foreground");
  const first = nav.querySelector("button")!;
  expect(first).toHaveClass("focus-visible:ring-muted-foreground");
  fireEvent.pointerEnter(first, { pointerType: "mouse" });
  expect(screen.getByRole("tooltip")).toHaveTextContent("Message 0");
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
  expect(nav.firstElementChild).toHaveStyle({ height: "40000px" });
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", "bottom"));
  expect(nav.querySelectorAll("button").length).toBeLessThan(30);
  nav.scrollTop = 24000;
  fireEvent.scroll(nav);
  expect(nav).toHaveAttribute("data-scroll-edges", "top bottom");
  expect(nav.querySelector('[data-chat-index-target="3000"]')).not.toBeNull();
  expect(nav.querySelectorAll("button").length).toBeLessThan(30);
  nav.scrollTop = 39920;
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
  expect(nav.scrollTop).toBe(39920);
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
  expect(navigate).toHaveBeenLastCalledWith(9);
  expect(nav.querySelector('[data-chat-index-target="9"]')).toHaveFocus();
});

it("follows transcript changes without fading the newly active tick", async () => {
  geometry();
  const { nav, scroll } = harness(100);
  scroll.current!.scrollTop = 5000;
  fireEvent.scroll(scroll.current!);
  await waitFor(() => expect(nav.querySelector('[data-chat-index-target="50"]')).toHaveAttribute("aria-current", "location"));
  expect(nav.scrollTop).toBe(348);
  await waitFor(() => expect(nav).toHaveAttribute("data-scroll-edges", "top bottom"));
});

it("keeps pointer selection on the dense tick slots during a drag", () => {
  geometry();
  const { nav, navigate } = harness(100);
  nav.setPointerCapture = vi.fn();
  fireEvent.pointerDown(nav.querySelector('[data-chat-index-target="4"]')!, { pointerId: 1, button: 0, clientY: 136 });
  fireEvent.pointerMove(nav, { pointerId: 1, clientY: 168 });
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
