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

  it("keeps the composer writable for an active typed task and sends steering", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(
      <ChatComposer
        {...baseProps}
        active
        allowActiveInput
        draftKey="conv:typed-task"
        onSend={onSend}
      />,
    );

    const input = screen.getByRole("textbox");
    expect(input).toBeEnabled();
    expect(input).toHaveAttribute("placeholder", "Steer active task...");
    fireEvent.change(input, {
      target: { value: "Keep the completed search results" },
    });
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Send steering instruction" }),
      );
    });

    expect(onSend).toHaveBeenCalledWith("Keep the completed search results");
    expect(baseProps.onStop).not.toHaveBeenCalled();
  });

  it("restores the typed text and draft when the send rejects [guard]", async () => {
    // A dead backend must not eat the message: a rejected send puts the text
    // back in the field and re-saves the draft, so retry is one keypress.
    const onSend = vi.fn().mockRejectedValue(new Error("backend down"));
    render(<ChatComposer {...baseProps} onSend={onSend} draftKey="conv:one" />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Keep me" },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    });

    expect(onSend).toHaveBeenCalledWith("Keep me");
    expect(screen.getByRole("textbox")).toHaveValue("Keep me");
    expect(screen.getByRole("textbox")).toBeEnabled();
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

/**
 * Park focus on the body, the way a browser does when the element holding it
 * is disabled. jsdom's `blur()` is a no-op on a disabled element, so focus a
 * throwaway node and remove it instead.
 */
function dropFocusToBody() {
  const scratch = document.createElement("button");
  document.body.append(scratch);
  scratch.focus();
  scratch.remove();
}

describe("ChatComposer focus", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("holds focus as soon as the chat is on screen", () => {
    render(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("follows the reader into a newly selected conversation, caret last", () => {
    useAssistantDraftStore
      .getState()
      .saveDraft("user-1", "conv:two", "Half-written");
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" focusRequest={1} />,
    );
    // Selecting a conversation means clicking its sidebar row, so the composer
    // has to take focus back off whatever was clicked.
    const elsewhere = document.createElement("button");
    document.body.append(elsewhere);
    elsewhere.focus();
    expect(elsewhere).toHaveFocus();

    rerender(
      <ChatComposer {...baseProps} draftKey="conv:two" focusRequest={2} />,
    );

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea).toHaveFocus();
    expect(textarea.selectionStart).toBe("Half-written".length);
    expect(textarea.selectionEnd).toBe("Half-written".length);
    elsewhere.remove();
  });

  it("still lands in the new conversation when the old one was mid-turn", () => {
    // Selecting away from a running turn changes `active` and the selection in
    // one render. Reading that as "a turn just ended" would apply the
    // leave-focus-alone rule to the sidebar row the reader just clicked.
    const { rerender } = render(
      <ChatComposer
        {...baseProps}
        active
        draftKey="conv:one"
        focusRequest={1}
      />,
    );
    const sidebarRow = document.createElement("button");
    document.body.append(sidebarRow);
    sidebarRow.focus();

    rerender(
      <ChatComposer {...baseProps} draftKey="conv:two" focusRequest={2} />,
    );

    expect(screen.getByRole("textbox")).toHaveFocus();
    sidebarRow.remove();
  });

  it("holds a focus request that arrives while a turn owns the field", () => {
    // A conversation selected while its own turn is still running cannot take
    // focus yet — the request has to survive until the field comes back.
    const { rerender } = render(
      <ChatComposer
        {...baseProps}
        active
        draftKey="conv:one"
        focusRequest={1}
      />,
    );
    rerender(
      <ChatComposer
        {...baseProps}
        active
        draftKey="conv:two"
        focusRequest={2}
      />,
    );
    expect(screen.getByRole("textbox")).not.toHaveFocus();

    rerender(
      <ChatComposer {...baseProps} draftKey="conv:two" focusRequest={2} />,
    );

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("ignores a draft-key change nobody asked for", () => {
    // Canonical-id repair, confirmed-stale cleanup and the first send's
    // migration all rewrite the URL on their own. None of them is a request to
    // start typing, so focus must stay where the reader put it.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:source" focusRequest={1} />,
    );
    const themeToggle = document.createElement("button");
    document.body.append(themeToggle);
    themeToggle.focus();

    rerender(
      <ChatComposer
        {...baseProps}
        draftKey="conv:canonical"
        focusRequest={1}
      />,
    );

    expect(themeToggle).toHaveFocus();
    expect(screen.getByRole("textbox")).not.toHaveFocus();
    themeToggle.remove();
  });

  it("still claims parked focus when the chat changes under browser Back", () => {
    // History navigation moves between conversations without remounting the
    // composer and without a request. Nobody is holding focus, so taking it is
    // free — and leaving the caret out of the composer is the whole complaint.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" focusRequest={1} />,
    );
    dropFocusToBody();

    rerender(
      <ChatComposer {...baseProps} draftKey="conv:two" focusRequest={1} />,
    );

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("does not restore focus to a reader who scrolled off during the turn", () => {
    // Sending with Enter means the composer held focus, so the turn owes it
    // back — until the reader scrolls the transcript or selects an answer.
    // Those leave `activeElement` on the body, exactly like the disable did,
    // so DOM focus alone cannot tell the two apart.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    expect(screen.getByRole("textbox")).toHaveFocus();

    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    dropFocusToBody();
    document.body.dispatchEvent(new Event("wheel", { bubbles: true }));

    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).not.toHaveFocus();
  });

  it("drops a held request once the reader goes somewhere else", () => {
    // A request queued behind a long turn is a minutes-old intent by the time
    // the field comes back; honouring it would yank focus off whatever the
    // reader picked up in the meantime.
    const { rerender } = render(
      <ChatComposer
        {...baseProps}
        active
        draftKey="conv:one"
        focusRequest={1}
      />,
    );
    rerender(
      <ChatComposer
        {...baseProps}
        active
        draftKey="conv:two"
        focusRequest={2}
      />,
    );

    const accountMenu = document.createElement("button");
    document.body.append(accountMenu);
    accountMenu.focus();
    accountMenu.dispatchEvent(new Event("pointerdown", { bubbles: true }));

    rerender(
      <ChatComposer {...baseProps} draftKey="conv:two" focusRequest={2} />,
    );

    expect(accountMenu).toHaveFocus();
    expect(screen.getByRole("textbox")).not.toHaveFocus();
    accountMenu.remove();
  });

  it("watches from the send, not from the turn going active", () => {
    // A first send waits on conversation creation and cache projection before
    // the transport says `running`, so `sending && !active` can last seconds.
    // Movement inside that window counts.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    expect(screen.getByRole("textbox")).toHaveFocus();

    rerender(<ChatComposer {...baseProps} sending draftKey="conv:one" />);
    document.body.dispatchEvent(new Event("wheel", { bubbles: true }));

    rerender(
      <ChatComposer {...baseProps} sending active draftKey="conv:one" />,
    );
    dropFocusToBody();
    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).not.toHaveFocus();
  });

  it("does not let one turn's verdict veto an unrelated later focus", () => {
    // The reader moves on mid-turn and the turn ends with focus still on the
    // control they moved to, so nothing consumes the verdict. It must not
    // survive to suppress the next parked draft-key move.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    const menu = document.createElement("button");
    document.body.append(menu);
    menu.focus();
    menu.dispatchEvent(new Event("pointerdown", { bubbles: true }));

    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);
    expect(menu).toHaveFocus();

    // Later: the menu closes, then browser Back moves to another chat.
    menu.remove();
    rerender(<ChatComposer {...baseProps} draftKey="conv:two" />);

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("does not count typing in the composer itself as moving on", () => {
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    const textarea = screen.getByRole("textbox");

    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    textarea.dispatchEvent(new Event("keydown", { bubbles: true }));
    dropFocusToBody();

    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("takes focus back when the turn that disabled it ends", () => {
    // A disabled textarea drops focus to the body; without this the reader has
    // to click back into the composer after every single answer.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    expect(screen.getByRole("textbox")).toHaveFocus();

    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    // jsdom does not drop focus when an element is disabled; browsers do.
    dropFocusToBody();
    expect(screen.getByRole("textbox")).not.toHaveFocus();

    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("does not grab focus from a reader who was not in the composer", () => {
    // The turn took nothing from them, so it has nothing to give back — a
    // reader scrolling the transcript stays where they are.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    dropFocusToBody();
    expect(screen.getByRole("textbox")).not.toHaveFocus();

    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).not.toHaveFocus();
  });

  it("leaves focus alone when a turn ends under someone's hands", () => {
    // Turns finish on their own schedule — often while the reader is part-way
    // through an approval card. Yanking focus out of that is worse than the
    // click it saves.
    const { rerender } = render(
      <ChatComposer {...baseProps} draftKey="conv:one" />,
    );
    expect(screen.getByRole("textbox")).toHaveFocus();

    rerender(<ChatComposer {...baseProps} active draftKey="conv:one" />);
    const approve = document.createElement("button");
    document.body.append(approve);
    approve.focus();

    rerender(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(approve).toHaveFocus();
    expect(screen.getByRole("textbox")).not.toHaveFocus();
    approve.remove();
  });

  it("does not raise the on-screen keyboard on a touch device", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn((query: string) => ({
        matches: query === "(pointer: coarse)",
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );

    render(<ChatComposer {...baseProps} draftKey="conv:one" />);

    expect(screen.getByRole("textbox")).not.toHaveFocus();
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
