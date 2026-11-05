import { StrictMode } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MessageCopyAction } from "./message-copy-action";

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
afterEach(() => {
  vi.unstubAllGlobals();
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard);
  else Reflect.deleteProperty(navigator, "clipboard");
});
function clipboard(value: unknown) {
  Object.defineProperty(navigator, "clipboard", { configurable: true, value });
}
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function clickCopy() {
  fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
}
class TestClipboardItem {
  readonly data: Record<string, Blob>;
  constructor(data: Record<string, Blob>) { this.data = data; }
}

describe("MessageCopyAction", () => {
  it("awaits the actual plain-text write and preserves literal user Markdown and code", async () => {
    const pending = deferred();
    const writeText = vi.fn(() => pending.promise);
    clipboard({ writeText });
    const text = "  **中文**\n\n- [link](https://example.test)\n```rust\n    let a = 1;\n```  ";
    render(<MessageCopyAction text={text} />);
    clickCopy();
    expect(writeText).toHaveBeenCalledExactlyOnceWith(text);
    expect(screen.queryByRole("button", { name: "Copied" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled();
    await act(async () => pending.resolve());
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("writes readable text and rich HTML, and waits for the rich write", async () => {
    const pending = deferred();
    const write = vi.fn<(items: TestClipboardItem[]) => Promise<void>>(() => pending.promise);
    const writeText = vi.fn();
    clipboard({ write, writeText });
    vi.stubGlobal("ClipboardItem", TestClipboardItem);
    render(<MessageCopyAction format="markdown" text={'**回答**\n\n[来源](https://example.test/long%20path)'} />);
    clickCopy();
    const item = write.mock.calls[0]![0][0] as TestClipboardItem;
    expect(Object.keys(item.data)).toEqual(["text/plain", "text/html"]);
    expect(await item.data["text/plain"]!.text()).toBe("回答\n\n来源");
    expect(await item.data["text/html"]!.text()).toContain('<strong>回答</strong>');
    expect(await item.data["text/html"]!.text()).toContain('href="https://example.test/long%20path"');
    expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled();
    await act(async () => pending.resolve());
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).not.toHaveBeenCalled();
  });

  it.each(["no write", "no ClipboardItem", "unsupported HTML", "rich format rejected"])("falls back to readable text when %s", async (reason) => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const write = vi.fn().mockRejectedValue(new DOMException("unsupported", "NotSupportedError"));
    clipboard({ writeText, ...(reason === "no write" ? {} : { write }) });
    vi.stubGlobal("ClipboardItem", reason === "no ClipboardItem" ? undefined : class extends TestClipboardItem {
      static supports() { return reason !== "unsupported HTML"; }
    });
    render(<MessageCopyAction format="markdown" text={'**Read**\n\n[label](https://example.test) with `**literal**`'} />);
    clickCopy();
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledExactlyOnceWith("Read\n\nlabel with **literal**");
    expect(write).toHaveBeenCalledTimes(reason === "rich format rejected" ? 1 : 0);
  });

  it("reports an async text rejection and permits a retry", async () => {
    const pending = deferred();
    const writeText = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(undefined);
    clipboard({ writeText });
    render(<MessageCopyAction text="body" />);
    clickCopy();
    await act(async () => pending.reject(new DOMException("denied", "NotAllowedError")));
    expect(screen.getByRole("status")).toHaveTextContent("Copy failed. Try Copy message again.");
    clickCopy();
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledTimes(2);
  });

  it("reports a denied rich write without claiming success or retrying another API", async () => {
    const pending = deferred();
    const write = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(undefined);
    const writeText = vi.fn();
    clipboard({ write, writeText });
    vi.stubGlobal("ClipboardItem", TestClipboardItem);
    render(<MessageCopyAction format="markdown" text="**body**" />);
    clickCopy();
    await act(async () => pending.reject(new DOMException("denied", "NotAllowedError")));
    expect(screen.getByRole("status")).toHaveTextContent("Copy failed");
    expect(screen.queryByRole("button", { name: "Copied" })).not.toBeInTheDocument();
    expect(writeText).not.toHaveBeenCalled();
    clickCopy();
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("reports failure when the clipboard is unavailable", async () => {
    clipboard(undefined);
    render(<MessageCopyAction format="markdown" text="body" />);
    clickCopy();
    expect(await screen.findByRole("status")).toHaveTextContent("Copy failed");
  });

  it.each(["resolve", "reject"] as const)("ignores an obsolete attempt's %s after the body changes", async (outcome) => {
    const old = deferred();
    const current = deferred();
    const writeText = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    clipboard({ writeText });
    const { rerender } = render(<MessageCopyAction text="old" />);
    clickCopy();
    rerender(<MessageCopyAction text="current" />);
    clickCopy();
    await act(async () => outcome === "resolve" ? old.resolve() : old.reject(new Error("old failure")));
    expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    await act(async () => current.resolve());
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("does not resurrect success after A → B → A or a format change", async () => {
    clipboard({ writeText: vi.fn().mockResolvedValue(undefined) });
    const { rerender } = render(<MessageCopyAction text="**A**" />);
    clickCopy();
    await screen.findByRole("button", { name: "Copied" });
    rerender(<MessageCopyAction text="B" />);
    rerender(<MessageCopyAction text="**A**" />);
    expect(screen.getByRole("button", { name: "Copy message" })).toBeEnabled();
    clickCopy();
    await screen.findByRole("button", { name: "Copied" });
    rerender(<MessageCopyAction text="**A**" format="markdown" />);
    expect(screen.getByRole("button", { name: "Copy message" })).toBeEnabled();
  });

  it("ignores completion after hiding or unmounting and works in StrictMode", async () => {
    const old = deferred();
    clipboard({ writeText: vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue(undefined) });
    const { rerender, unmount } = render(<StrictMode><MessageCopyAction text="body" /></StrictMode>);
    clickCopy();
    rerender(<StrictMode><MessageCopyAction text=" " /></StrictMode>);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    rerender(<StrictMode><MessageCopyAction text="body" /></StrictMode>);
    await act(async () => old.resolve());
    expect(screen.getByRole("button", { name: "Copy message" })).toBeEnabled();
    clickCopy();
    await screen.findByRole("button", { name: "Copied" });
    unmount();
  });
});
