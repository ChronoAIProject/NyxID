import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockCopyToClipboard = vi.fn();
const mockToastSuccess = vi.fn();

vi.mock("@/lib/utils", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/utils")>("@/lib/utils");
  return { ...actual, copyToClipboard: mockCopyToClipboard };
});

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => mockToastSuccess(...args),
    error: vi.fn(),
  },
}));

import { DocMarkdown } from "./docs-markdown";

const MARKDOWN = [
  "Some intro prose.",
  "",
  "```bash",
  'nyxid service add llm-openrouter \\',
  '  --credential-env OPENROUTER_KEY',
  "```",
  "",
  "Inline `code span` stays button-free.",
].join("\n");

beforeEach(() => {
  vi.clearAllMocks();
  mockCopyToClipboard.mockResolvedValue(undefined);
});

describe("DocMarkdown — code block copy button", () => {
  it("copies the fenced snippet text to the clipboard", async () => {
    const user = userEvent.setup();
    render(<DocMarkdown markdown={MARKDOWN} baseHref="/docs/ai/guides/" />);

    const button = screen.getByRole("button", { name: /copy code snippet/i });
    await user.click(button);

    await waitFor(() => expect(mockCopyToClipboard).toHaveBeenCalledTimes(1));
    const copied = mockCopyToClipboard.mock.calls[0]?.[0] as string;
    expect(copied).toContain("nyxid service add llm-openrouter");
    expect(copied).toContain("--credential-env OPENROUTER_KEY");
    // The trailing newline a fenced block renders with is trimmed.
    expect(copied.endsWith("\n")).toBe(false);
    expect(mockToastSuccess).toHaveBeenCalledWith("Copied to clipboard");
  });

  it("renders one button per fenced block and none for inline code", () => {
    const twoBlocks = "```bash\nfirst\n```\n\ntext with `inline` code\n\n```bash\nsecond\n```";
    render(<DocMarkdown markdown={twoBlocks} baseHref="/docs/ai/guides/" />);

    expect(
      screen.getAllByRole("button", { name: /copy code snippet/i }),
    ).toHaveLength(2);
  });
});
