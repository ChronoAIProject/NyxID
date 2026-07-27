import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ChatComposer } from "./chat-composer";

function renderComposer() {
  const onSend = vi.fn().mockResolvedValue(undefined);
  render(
    <ChatComposer
      active={false}
      sending={false}
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
