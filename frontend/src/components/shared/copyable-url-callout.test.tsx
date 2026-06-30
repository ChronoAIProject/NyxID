import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CopyableUrlCallout } from "./copyable-url-callout";

const mocks = vi.hoisted(() => ({
  copyToClipboard: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/lib/utils", async () => {
  const actual = await vi.importActual<typeof import("@/lib/utils")>(
    "@/lib/utils",
  );
  return {
    ...actual,
    copyToClipboard: mocks.copyToClipboard,
  };
});

vi.mock("sonner", () => ({
  toast: {
    success: mocks.toastSuccess,
    error: mocks.toastError,
  },
}));

describe("CopyableUrlCallout", () => {
  beforeEach(() => {
    mocks.copyToClipboard.mockReset();
    mocks.copyToClipboard.mockResolvedValue(undefined);
    mocks.toastSuccess.mockReset();
    mocks.toastError.mockReset();
  });

  it("renders the label, URL, and description when provided", () => {
    render(
      <CopyableUrlCallout
        label="Webhook URL"
        url="https://api.example.test/api/v1/webhooks/channel/lark/abc-123"
        description="Paste this into your bot platform's Event Subscriptions / webhook settings"
      />,
    );

    expect(screen.getByText("Webhook URL")).toBeInTheDocument();
    expect(
      screen.getByText(
        "https://api.example.test/api/v1/webhooks/channel/lark/abc-123",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Paste this into your bot platform's Event Subscriptions / webhook settings",
      ),
    ).toBeInTheDocument();
  });

  it("omits the description block when not provided", () => {
    render(<CopyableUrlCallout label="Webhook URL" url="https://api.example.test/wh" />);
    expect(screen.getByText("Webhook URL")).toBeInTheDocument();
    expect(screen.getByText("https://api.example.test/wh")).toBeInTheDocument();
    // No muted description paragraph — only the label + URL + copy button.
    expect(screen.queryByText(/Paste this/i)).not.toBeInTheDocument();
  });

  it("copy button writes the URL to the clipboard and fires a success toast", async () => {
    const user = userEvent.setup();
    const url = "https://api.example.test/api/v1/webhooks/channel/telegram/bot-9";
    render(<CopyableUrlCallout label="Webhook URL" url={url} />);

    await user.click(screen.getByRole("button", { name: "Copy Webhook URL" }));

    expect(mocks.copyToClipboard).toHaveBeenCalledWith(url);
    expect(mocks.toastError).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(mocks.toastSuccess).toHaveBeenCalledWith("Webhook URL copied");
    });
  });

  it("fires a toast.error when the clipboard write rejects", async () => {
    const user = userEvent.setup();
    mocks.copyToClipboard.mockRejectedValue(new Error("clipboard blocked"));
    render(<CopyableUrlCallout label="Webhook URL" url="https://api.example.test/x" />);

    await user.click(screen.getByRole("button", { name: "Copy Webhook URL" }));

    await waitFor(() => {
      expect(mocks.toastError).toHaveBeenCalledWith("Failed to copy");
    });
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it("renders the 'Learn more →' link only when docsHref is provided", () => {
    const { rerender } = render(
      <CopyableUrlCallout label="Webhook URL" url="https://api.example.test/wh" />,
    );
    expect(screen.queryByRole("link", { name: /Learn more/i })).not.toBeInTheDocument();

    rerender(
      <CopyableUrlCallout
        label="Webhook URL"
        url="https://api.example.test/wh"
        docsHref="https://core.telegram.org/bots/webhooks"
      />,
    );
    const link = screen.getByRole("link", { name: /Learn more/i });
    expect(link).toHaveAttribute(
      "href",
      "https://core.telegram.org/bots/webhooks",
    );
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });
});
