import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { ChatComposer } from "./chat-composer";

const baseProps = {
  active: false,
  sending: false,
  ownerUserId: "user-1",
  onSend: vi.fn().mockResolvedValue(undefined),
  onStop: vi.fn().mockResolvedValue(undefined),
};

describe("ChatComposer drafts", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
    baseProps.onSend.mockClear();
    baseProps.onStop.mockClear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("debounces typing into the active chat draft", () => {
    render(<ChatComposer {...baseProps} draftKey="conv:one" />);
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "Half a question" } });

    act(() => vi.advanceTimersByTime(299));
    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe("");
    act(() => vi.advanceTimersByTime(1));
    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe(
      "Half a question",
    );
  });

  it("flushes the outgoing chat and restores the incoming chat without bleed", () => {
    const store = useAssistantDraftStore.getState();
    store.saveDraft("user-1", "conv:one", "Original one");
    store.saveDraft("user-1", "conv:two", "Draft two");
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    const input = screen.getByRole("textbox");
    expect(input).toHaveValue("Original one");

    fireEvent.change(input, { target: { value: "Edited one" } });
    rerender(<ChatComposer {...baseProps} draftKey="conv:two" />);

    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe(
      "Edited one",
    );
    expect(screen.getByRole("textbox")).toHaveValue("Draft two");
  });

  it("clears the draft at dispatch and does not restore it after stop", async () => {
    useAssistantDraftStore
      .getState()
      .saveDraft("user-1", "conv:one", "Send this");
    render(<ChatComposer {...baseProps} draftKey="conv:one" />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    });

    expect(baseProps.onSend).toHaveBeenCalledWith("Send this");
    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe("");
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  it("flushes immediately on unmount", () => {
    const { unmount } = render(
      <ChatComposer {...baseProps} draftKey="screen:/keys" />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Uncreated chat draft" },
    });
    unmount();

    expect(useAssistantDraftStore.getState().getDraft("screen:/keys")).toBe(
      "Uncreated chat draft",
    );
  });

  it("does not resurrect a draft when the store is cleared before unmount", () => {
    const { unmount } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Private draft" },
    });

    useAssistantDraftStore.getState().clear();
    unmount();

    expect(useAssistantDraftStore.getState()).toMatchObject({
      ownerUserId: null,
      drafts: {},
    });
    expect(localStorage.getItem("nyxid.assistant_drafts")).toBeNull();
  });

  it("does not restore the prior owner when dispatch races with logout", async () => {
    render(<ChatComposer {...baseProps} draftKey="conv:one" />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Private draft" },
    });
    useAssistantDraftStore.getState().clear();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    });

    expect(useAssistantDraftStore.getState()).toMatchObject({
      ownerUserId: null,
      drafts: {},
    });
    expect(localStorage.getItem("nyxid.assistant_drafts")).toBeNull();
  });

  it("flushes immediately before the page unloads", () => {
    render(<ChatComposer {...baseProps} draftKey="screen:/nodes" />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Persist before reload" },
    });
    window.dispatchEvent(new Event("beforeunload"));

    expect(useAssistantDraftStore.getState().getDraft("screen:/nodes")).toBe(
      "Persist before reload",
    );
  });
});

function renderImeComposer() {
  const onSend = vi.fn().mockResolvedValue(undefined);
  render(
    <ChatComposer
      active={false}
      sending={false}
      ownerUserId="user-1"
      draftKey="conv:ime"
      onSend={onSend}
      onStop={vi.fn().mockResolvedValue(undefined)}
    />,
  );
  return {
    composer: screen.getByPlaceholderText("Message NyxID Assistant..."),
    onSend,
  };
}

describe("ChatComposer IME handling", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  });

  it("lets Enter commit an active IME composition without sending", async () => {
    const { composer, onSend } = renderImeComposer();
    fireEvent.change(composer, { target: { value: "workfl" } });

    fireEvent.compositionStart(composer);
    fireEvent.keyDown(composer, { key: "Enter", code: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
    expect(composer).toHaveValue("workfl");

    fireEvent.compositionEnd(composer);
    fireEvent.keyDown(composer, { key: "Enter", code: "Enter" });

    await waitFor(() => expect(onSend).toHaveBeenCalledWith("workfl"));
    expect(composer).toHaveValue("");
  });

  it("also honors the native composing flag when composition events are unavailable", () => {
    const { composer, onSend } = renderImeComposer();
    fireEvent.change(composer, { target: { value: "workflow" } });

    fireEvent.keyDown(composer, {
      key: "Enter",
      code: "Enter",
      isComposing: true,
    });

    expect(onSend).not.toHaveBeenCalled();
    expect(composer).toHaveValue("workflow");
  });

  it("does not send legacy IME key events reported with key code 229", () => {
    const { composer, onSend } = renderImeComposer();
    fireEvent.change(composer, { target: { value: "workflow" } });

    fireEvent.keyDown(composer, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
    });

    expect(onSend).not.toHaveBeenCalled();
    expect(composer).toHaveValue("workflow");
  });
});

let resizeCallbacks: ResizeObserverCallback[];

class ResizeObserverMock {
  constructor(callback: ResizeObserverCallback) {
    resizeCallbacks.push(callback);
  }

  observe() {}

  disconnect() {}
}

function renderMeasuredComposer() {
  render(
    <ChatComposer
      active={false}
      sending={false}
      ownerUserId="user-1"
      draftKey="conv:layout"
      onSend={vi.fn().mockResolvedValue(undefined)}
      onStop={vi.fn().mockResolvedValue(undefined)}
    />,
  );

  const textarea = screen.getByRole("textbox");
  const button = screen.getByRole("button", { name: "Send message" });
  const controls = button.parentElement;
  const composer = controls?.parentElement;
  const textMeasure = composer?.querySelector(":scope > span[aria-hidden]");
  if (!controls || !composer || !(textMeasure instanceof HTMLElement)) {
    throw new Error("Composer measurement elements were not rendered");
  }

  return { button, composer, controls, textarea, textMeasure };
}

function setComposerMetrics({
  composer,
  controls,
  textMeasure,
  composerWidth = 400,
  controlsWidth = 32,
  textWidth,
}: {
  composer: HTMLElement;
  controls: HTMLElement;
  textMeasure: HTMLElement;
  composerWidth?: number;
  controlsWidth?: number;
  textWidth: number;
}) {
  composer.style.padding = "6px";
  composer.style.gap = "6px";
  Object.defineProperty(composer, "clientWidth", {
    configurable: true,
    value: composerWidth,
  });
  vi.spyOn(controls, "getBoundingClientRect").mockReturnValue({
    ...controls.getBoundingClientRect(),
    width: controlsWidth,
  });
  vi.spyOn(textMeasure, "getBoundingClientRect").mockReturnValue({
    ...textMeasure.getBoundingClientRect(),
    width: textWidth,
  });
}

describe("ChatComposer measured layout", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
    resizeCallbacks = [];
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("uses a top-aligned row without the broker caption", () => {
    const { button, composer, textarea } = renderMeasuredComposer();

    expect(
      screen.queryByText(
        "Every agent action is brokered, scoped, and audit-logged by NyxID.",
      ),
    ).not.toBeInTheDocument();
    expect(composer).toHaveClass("ml-[30px]", "flex", "items-start");
    expect(button).toHaveClass("h-8");
    expect(textarea).toHaveClass("min-h-8", "px-0");
  });

  it("keeps a short single-line draft and its action inline", () => {
    const parts = renderMeasuredComposer();
    setComposerMetrics({ ...parts, textWidth: 120 });

    fireEvent.change(parts.textarea, { target: { value: "Short draft" } });

    expect(parts.composer).toHaveClass("items-start");
    expect(parts.composer).not.toHaveClass("flex-col");
    expect(parts.controls).not.toHaveClass("self-end");
  });

  it("moves the action below at 95% of the remaining inline width", () => {
    const parts = renderMeasuredComposer();
    setComposerMetrics({ ...parts, textWidth: 350 });

    fireEvent.change(parts.textarea, {
      target: { value: "Draft reaching the remaining-width threshold" },
    });

    expect(parts.composer).toHaveClass("flex-col", "items-stretch");
    expect(parts.controls).toHaveClass("self-end");
  });

  it("returns inline when text falls below the threshold", () => {
    const parts = renderMeasuredComposer();
    let textWidth = 350;
    setComposerMetrics({ ...parts, textWidth });
    vi.mocked(parts.textMeasure.getBoundingClientRect).mockImplementation(
      () => ({ width: textWidth }) as DOMRect,
    );

    fireEvent.change(parts.textarea, { target: { value: "Long draft" } });
    expect(parts.composer).toHaveClass("flex-col");

    textWidth = 80;
    fireEvent.change(parts.textarea, { target: { value: "Short" } });
    expect(parts.composer).not.toHaveClass("flex-col");
    expect(parts.composer).toHaveClass("items-start");
  });

  it("uses multiline layout immediately for an explicit newline", () => {
    const parts = renderMeasuredComposer();
    setComposerMetrics({ ...parts, textWidth: 40 });

    fireEvent.change(parts.textarea, {
      target: { value: "Line one\nLine two" },
    });

    expect(parts.composer).toHaveClass("flex-col", "items-stretch");
  });

  it("recalculates remaining text width when controls grow", () => {
    const parts = renderMeasuredComposer();
    let controlsWidth = 32;
    setComposerMetrics({ ...parts, controlsWidth, textWidth: 310 });
    vi.mocked(parts.controls.getBoundingClientRect).mockImplementation(
      () => ({ width: controlsWidth }) as DOMRect,
    );

    fireEvent.change(parts.textarea, { target: { value: "Draft" } });
    expect(parts.composer).not.toHaveClass("flex-col");

    controlsWidth = 80;
    act(() => {
      for (const callback of resizeCallbacks) {
        callback([], {} as ResizeObserver);
      }
    });
    expect(parts.composer).toHaveClass("flex-col");
  });

  it("caps long drafts and fades only scrollable textarea edges", () => {
    const { textarea } = renderMeasuredComposer();
    textarea.style.lineHeight = "20px";
    textarea.style.paddingTop = "4px";
    textarea.style.paddingBottom = "4px";
    Object.defineProperties(textarea, {
      clientHeight: { configurable: true, value: 88 },
      scrollHeight: { configurable: true, value: 260 },
      scrollTop: { configurable: true, writable: true, value: 0 },
    });

    fireEvent.change(textarea, { target: { value: "A long draft" } });
    expect(textarea).toHaveStyle({ height: "88px", overflowY: "auto" });

    const fades = textarea.parentElement?.querySelectorAll(
      ':scope > [aria-hidden="true"]',
    );
    expect(fades).toBeDefined();
    if (!fades) return;
    expect(fades).toHaveLength(2);
    expect(fades[0]).toHaveClass("opacity-0");
    expect(fades[1]).toHaveClass("opacity-100");

    textarea.scrollTop = 40;
    fireEvent.scroll(textarea);
    expect(fades[0]).toHaveClass("opacity-100");
    expect(fades[1]).toHaveClass("opacity-100");
  });
});
