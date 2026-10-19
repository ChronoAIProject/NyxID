import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MessageCopyAction } from "./message-copy-action";

describe("MessageCopyAction", () => {
  it("awaits the actual clipboard result and preserves Markdown and code", async () => {
    let resolve!: () => void;
    const writeText = vi.fn(() => new Promise<void>((done) => { resolve = done; }));
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const text = "中文\n\n- [link](https://example.test)\n```rust\n    let a = 1;\n```";
    render(<MessageCopyAction text={text} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
    expect(writeText).toHaveBeenCalledWith(text);
    expect(screen.queryByRole("button", { name: "Copied" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled();
    await act(async () => resolve());
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });
  it("reports rejection and permits a retry", async () => {
    const writeText = vi.fn().mockRejectedValueOnce(new Error("denied")).mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<MessageCopyAction text="body" />);
    fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
    expect(await screen.findByText("Copy failed. Try Copy message again.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledTimes(2);
  });
});
