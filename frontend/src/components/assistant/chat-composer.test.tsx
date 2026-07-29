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

function renderComposer() {
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
    const { composer, onSend } = renderComposer();
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
    const { composer, onSend } = renderComposer();
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
    const { composer, onSend } = renderComposer();
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
