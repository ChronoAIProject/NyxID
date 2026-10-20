import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ChannelConnectLinkPage } from "./channel-connect-link";
import type { ChannelConnectLink } from "@/schemas/channel-connect-links";

const mocks = vi.hoisted(() => ({
  data: {} as ChannelConnectLink,
  error: null as Error | null,
  refresh: vi.fn(),
  navigate: vi.fn(),
  mutate: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ token: "nyx_bcl_example" }),
  useNavigate: () => mocks.navigate,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({
    user: { id: "owner", email: "owner@example.com" },
    isAuthenticated: true,
    isLoading: false,
  }),
}));
vi.mock("@/hooks/use-channel-connect-link", async (original) => ({
  ...(await original<object>()),
  useChannelConnectLink: () => ({
    status: { data: mocks.data, error: mocks.error },
    preview: {},
    refresh: mocks.refresh,
    action: { mutate: mocks.mutate, isPending: false, error: null },
  }),
}));
vi.mock("@/components/channels/channel-connection-shell", () => ({
  ChannelConnectionShell: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
}));
vi.mock("@/components/channels/channel-bot-setup", async () => {
  const { useState } = await import("react");
  return {
    ChannelBotSetup: ({
      onContinue,
      onComplete,
      defaultLabel,
      defaultOrgId,
    }: {
      onContinue: () => void;
      onComplete: () => void;
      defaultLabel: string;
      defaultOrgId: string | null;
    }) => {
      const [secret, setSecret] = useState<string | null>(null);
      return (
        <div>
          <span>
            {defaultLabel}:{defaultOrgId ?? "personal"}
          </span>
          <button onClick={() => { setSecret("one-time-verification-secret"); onComplete(); }}>
            Complete setup
          </button>
          {secret && <p>{secret}</p>}
          <button onClick={onContinue}>Acknowledge secret</button>
        </div>
      );
    },
  };
});

beforeEach(() => {
  mocks.error = null;
  mocks.refresh.mockResolvedValue(undefined);
  mocks.data = {
    id: "link",
    status: "pending",
    platform: "discord",
    label: "Support",
    owner_id: "org-owner",
    requested_by: "My app",
    expires_at: "2026-09-30T00:00:00Z",
    bot_id: null,
    connection_id: null,
    telegram_request_id: null,
    callback_url: null,
    last_error: null,
    delivery_status: null,
  };
});

it("keeps the setup and one-time secret mounted when completion status arrives", async () => {
  const view = render(<ChannelConnectLinkPage />);
  expect(screen.getByText("Support:org-owner")).toBeInTheDocument();
  fireEvent.click(screen.getByText("Complete setup"));
  mocks.data = {
    ...mocks.data,
    status: "completed",
    bot_id: "bot",
    callback_url: "https://example.com/return?status=completed",
  };
  view.rerender(<ChannelConnectLinkPage />);
  expect(screen.getByText("one-time-verification-secret")).toBeInTheDocument();
  expect(screen.queryByText("Support is connected.")).not.toBeInTheDocument();
  fireEvent.click(screen.getByText("Acknowledge secret"));
  await waitFor(() =>
    expect(screen.getByText("Support is connected.")).toBeInTheDocument(),
  );
  expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument();
});

it("recovers a saved bot without rendering another creation form", () => {
  mocks.data = { ...mocks.data, bot_id: "bot", last_error: "setup_incomplete" };
  render(<ChannelConnectLinkPage />);
  expect(screen.queryByText("Complete setup")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry setup" }));
  expect(mocks.mutate).toHaveBeenCalledWith("retry");
});

it("explains when another administrator must resume Telegram consent", () => {
  mocks.data = { ...mocks.data, platform: "telegram-new", telegram_request_id: "request", telegram_requires_original_actor: true };
  render(<ChannelConnectLinkPage />);
  expect(screen.getByText(/Another administrator started this Telegram setup/)).toBeInTheDocument();
  expect(screen.queryByText("Complete setup")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry setup" })).not.toBeInTheDocument();
});

it("shows terminal status directly when reopening a completed link", () => {
  mocks.data = { ...mocks.data, status: "completed", bot_id: "bot" };
  render(<ChannelConnectLinkPage />);
  expect(screen.getByText("Support is connected.")).toBeInTheDocument();
  expect(screen.queryByText("Complete setup")).not.toBeInTheDocument();
});

it("preserves the one-time result through failed status refresh and recovery", () => {
  const view = render(<ChannelConnectLinkPage />);
  fireEvent.click(screen.getByText("Complete setup"));
  mocks.data = { ...mocks.data, status: "completed", bot_id: "bot" };
  mocks.error = new Error("Temporary network failure");
  view.rerender(<ChannelConnectLinkPage />);
  expect(screen.getByText("one-time-verification-secret")).toBeInTheDocument();
  expect(screen.getByText("Temporary network failure")).toBeInTheDocument();
  mocks.error = null;
  view.rerender(<ChannelConnectLinkPage />);
  expect(screen.getByText("one-time-verification-secret")).toBeInTheDocument();
});

it("does not discard an in-flight wizard because polling returned an older setup error", () => {
  const view = render(<ChannelConnectLinkPage />);
  mocks.data = { ...mocks.data, bot_id: "bot", last_error: "setup_incomplete" };
  view.rerender(<ChannelConnectLinkPage />);
  fireEvent.click(screen.getByText("Complete setup"));
  expect(screen.getByText("one-time-verification-secret")).toBeInTheDocument();
});

it("keeps an unacknowledged setup secret if background recovery later expires", () => {
  const view = render(<ChannelConnectLinkPage />);
  fireEvent.click(screen.getByText("Complete setup"));
  mocks.data = { ...mocks.data, status: "expired", bot_id: "bot" };
  view.rerender(<ChannelConnectLinkPage />);
  expect(screen.getByText("one-time-verification-secret")).toBeInTheDocument();
});
