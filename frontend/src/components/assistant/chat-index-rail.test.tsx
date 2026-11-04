import { createRef } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ChatIndexRail } from "./chat-index-rail";
import type { ChatMessage } from "@/lib/assistant/chat-types";

const messages: ChatMessage[] = Array.from({ length: 80 }, (_, index) => ({ id: `message-${index}`, role: index % 2 ? "assistant" : "user", content: `Message ${index}`, timestamp: 0, status: "complete" }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("fits uniform text-scaled slots below the toolbar and updates when the toolbar wraps", () => {
  const resizes: (() => void)[] = [];
  let pitch = 14, bottom = 70;
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resizes.push(callback); } observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { top: 20, bottom: this.hasAttribute("data-chat-header-controls") ? bottom : 520, height: this.hasAttribute("data-chat-index-pitch") ? pitch : 500 } as DOMRect;
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(450);
  const scroll = createRef<HTMLDivElement>();
  render(<div data-chat-content-layout=""><div data-chat-header-controls="" /><div><div ref={scroll} /><ChatIndexRail messages={messages} scrollRef={scroll} bottomInset={90} onNavigate={vi.fn()} /></div></div>);
  const nav = screen.getByRole("navigation", { name: "Conversation index" });
  expect(nav.parentElement).toHaveStyle({ top: "58px" });
  expect(nav.querySelectorAll("button")).toHaveLength(32);
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "14px" });
  pitch = 21; bottom = 110;
  act(() => resizes[0]!());
  expect(nav.parentElement).toHaveStyle({ top: "98px" });
  expect(nav.querySelectorAll("button")).toHaveLength(21);
  for (const tick of nav.querySelectorAll("button")) expect(tick).toHaveStyle({ height: "21px" });
});

it("keeps active state, two tick lengths, hover previews, click and keyboard jumps", async () => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(450);
  const navigate = vi.fn();
  const scroll = createRef<HTMLDivElement>();
  render(<div><div ref={scroll} /><ChatIndexRail messages={messages.slice(0, 4)} scrollRef={scroll} bottomInset={90} onNavigate={navigate} /></div>);
  const nav = screen.getByRole("navigation", { name: "Conversation index" });
  await waitFor(() => expect(nav.querySelector('[aria-current="location"]')).not.toBeNull());
  expect(nav.querySelector('[aria-current="location"] > span')).toHaveClass("bg-success");
  expect(nav.querySelector('[data-message-role="user"] > span')).toHaveClass("w-2");
  expect(nav.querySelector('[data-message-role="assistant"] > span')).toHaveClass("w-4");
  const first = nav.querySelector("button")!;
  fireEvent.pointerEnter(first, { pointerType: "mouse" });
  expect(screen.getByRole("tooltip")).toHaveTextContent("Message 0");
  fireEvent.click(first, { detail: 0 });
  expect(navigate).toHaveBeenLastCalledWith(0);
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(navigate).toHaveBeenLastCalledWith(1);
  expect(nav.querySelector('[data-chat-index-target="1"]')).toHaveFocus();
});
