import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// vi.mock factories are hoisted above const initializers — declare the
// mocks with vi.hoisted so the factory can't hit the TDZ when another
// module in the graph imports @/lib/utils first (repo pattern, see
// key-detail.test.tsx).
const { mockCopyToClipboard, mockToastSuccess } = vi.hoisted(() => ({
  mockCopyToClipboard: vi.fn(),
  mockToastSuccess: vi.fn(),
}));

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

  it("skips untagged fences (ASCII diagrams) and text-family languages", () => {
    // Mirrors the broker-model concept page: an untagged fence holding a
    // sequence diagram, plus a ```text block — neither is terminal-paste
    // material, so neither gets a copy affordance.
    const markdown = [
      "```",
      "Caller ──▶ NyxID Proxy ──▶ Downstream API",
      "  │            │                │",
      "```",
      "",
      "```text",
      "plain prose sample",
      "```",
      "",
      "```bash",
      "nyxid service list",
      "```",
    ].join("\n");
    render(<DocMarkdown markdown={markdown} baseHref="/docs/shared/concepts/" />);

    // Only the bash block is copyable.
    expect(
      screen.getAllByRole("button", { name: /copy code snippet/i }),
    ).toHaveLength(1);
  });
});

describe("published getting-started section links", () => {
  it.each([
    ["web/getting-started/first-connection", [
      "hosted-recommended-for-first-time-users", "1-sign-in", "2-add-an-ai-service",
      "3-create-a-nyxid-agent-key", "4-run-the-verification-curl", "self-host", "troubleshooting", "next",
    ]],
    ["ai/getting-started/connect-your-agent", [
      "install-the-cli", "authenticate-you-the-human", "authorize-an-agent-profile-without-transferring-a-secret",
      "create-an-agent-key-for-the-agent-to-use", "wire-your-mcp-client", "claude-code", "cursor", "codex",
      "generate-the-config-automatically", "what-you-see-after-wiring-mcp", "next-steps",
    ]],
    ["ai/getting-started/first-agent-call", [
      "prerequisites", "step-1-add-the-service", "step-2-make-a-proxied-http-request",
      "step-3-verify-the-proxy-flow", "step-4-invoke-as-an-mcp-tool", "step-5-check-the-audit-log", "whats-next",
    ]],
    ["cli/getting-started/first-connection", [
      "1-set-the-provider-credential", "2-add-the-service-from-the-catalog", "3-copy-the-returned-slug",
      "4-verify-with-a-proxied-request", "if-it-errors", "next",
    ]],
  ] as const)("renders the published anchors for %s", (slug, publishedIds) => {
    const markdown = readFileSync(resolve(import.meta.dirname, "../../../../docs/site", `${slug}.md`), "utf8");
    const { container } = render(<DocMarkdown markdown={markdown} baseHref="/docs/" />);
    const ids = Array.from(container.querySelectorAll("h2[id], h3[id]"), (heading) => heading.id);
    expect(ids).toEqual(expect.arrayContaining([...publishedIds]));
  });
});
