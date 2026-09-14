import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { TelegramNew } from "./telegram-new";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

const { mockGet, mockPost, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockDelete: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, delete: mockDelete },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "actor-1" } }),
}));

const request: TelegramNewRequest = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
  status: "waiting_telegram",
  revision: 1,
  owner_user_id: "actor-1",
  label: "Support",
  expires_at: "2026-09-11T10:00:00Z",
  telegram_bot_id: null,
  bot_username: null,
  channel_bot_id: null,
};
const ready: TelegramNewRequest = {
  ...request,
  status: "ready",
  revision: 6,
  telegram_bot_id: "900",
  bot_username: "SupportBot",
};
const configuration = (value: TelegramNewRequest | null) => ({
  available: true,
  manager_username: "NyxSetupBot",
  request: value,
});

function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const onConnected = vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <TelegramNew label="Support" orgId={null} onConnected={onConnected} />
    </QueryClientProvider>,
  );
  return { ...view, client, onConnected };
}

beforeEach(() => vi.resetAllMocks());

it("prepares a saved request before exposing a separate native launch link, then confirms the approved bot", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(configuration(null));
  mockPost.mockImplementation(async (url: string) => {
    if (url.endsWith("/connect")) {
      const result = {
        ...ready,
        status: "connected",
        channel_bot_id: request.id,
      };
      mockGet.mockResolvedValue(configuration(null));
      return result;
    }
    mockGet.mockResolvedValue(configuration(request));
    return { request, launch_url: "https://t.me/NyxSetupBot?start=challenge" };
  });
  const view = setup();
  expect(
    screen.queryByRole("link", { name: "Open Telegram" }),
  ).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole("button", { name: "Save and continue" }),
  );
  expect(
    await screen.findByRole("link", { name: "Open Telegram" }),
  ).toHaveAttribute("href", "https://t.me/NyxSetupBot?start=challenge");
  expect(mockPost).toHaveBeenCalledWith("/channel-bots/telegram-new", {
    label: "Support",
  });
  expect(
    screen.queryByRole("button", { name: "Connect bot" }),
  ).not.toBeInTheDocument();
  mockGet.mockResolvedValue(configuration(ready));
  await act(() =>
    view.client.invalidateQueries({ queryKey: ["telegram-new"] }),
  );
  await user.click(await screen.findByRole("button", { name: "Connect bot" }));
  await waitFor(() =>
    expect(view.onConnected).toHaveBeenCalledWith(request.id),
  );
  expect(mockPost).toHaveBeenCalledWith(
    `/channel-bots/telegram-new/requests/${request.id}/connect`,
    { telegram_bot_id: "900", revision: 6 },
  );
});

it("resumes a pending bot after reopening, keeps the saved destination and offers retry", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(
    configuration({
      ...ready,
      status: "provisioning",
      owner_user_id: "original-org",
      channel_bot_id: request.id,
    }),
  );
  mockPost.mockRejectedValue(new Error("Temporary Telegram failure"));
  setup();
  expect(
    await screen.findByText("NyxID account ID: original-org"),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/This setup uses the account you selected earlier/),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Retry connection" }));
  expect(
    await screen.findByText("Temporary Telegram failure"),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Retry connection" }),
  ).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Cancel setup" }),
  ).not.toBeInTheDocument();
});

it("cancels an existing creation and lets the user start again", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(configuration(request));
  mockDelete.mockImplementation(async () => {
    mockGet.mockResolvedValue(configuration(null));
  });
  setup();
  await user.click(await screen.findByRole("button", { name: "Cancel setup" }));
  expect(
    await screen.findByRole("button", { name: "Save and continue" }),
  ).toBeEnabled();
  expect(mockDelete).toHaveBeenCalledWith(
    `/channel-bots/telegram-new/requests/${request.id}`,
  );
});

it("explains the administrator setup when no manager is configured", async () => {
  mockGet.mockResolvedValue({
    ...configuration(null),
    available: false,
    manager_username: null,
  });
  setup();
  expect(
    await screen.findByText(/administrator needs to configure a manager/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Save and continue" }),
  ).not.toBeInTheDocument();
});

it("shows the current Telegram action and lets the user check progress after returning", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(
    configuration({ ...request, status: "waiting_bot" }),
  );
  setup();
  expect(
    await screen.findByText(
      "You have started the setup chat. Tap Create bot there to make your bot.",
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("list", { name: "Telegram setup steps" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /This is the setup chat that helps you create your own bot/,
    ),
  ).toBeInTheDocument();
  mockGet.mockResolvedValue(
    configuration({ ...request, status: "waiting_consent" }),
  );
  await user.click(screen.getByRole("button", { name: "Check progress" }));
  expect(
    await screen.findByText(
      "Your bot has been created. Open the setup chat and tap Approve this bot.",
    ),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Connect bot" }),
  ).not.toBeInTheDocument();
});

it("regenerates a launch link for the saved request after a full remount", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(configuration(request));
  mockPost.mockResolvedValue({
    request,
    launch_url: "https://t.me/NyxSetupBot?start=fresh-challenge",
  });
  const first = setup();
  await screen.findByRole("button", { name: "Get Telegram link" });
  first.unmount();
  first.client.clear();
  setup();
  expect(
    screen.queryByRole("link", { name: "Open Telegram" }),
  ).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole("button", { name: "Get Telegram link" }),
  );
  expect(
    await screen.findByRole("link", { name: "Open Telegram" }),
  ).toHaveAttribute("href", "https://t.me/NyxSetupBot?start=fresh-challenge");
  expect(mockPost).toHaveBeenCalledExactlyOnceWith(
    `/channel-bots/telegram-new/requests/${request.id}/launch`,
  );
  expect(
    screen.queryByText(
      "Save your details first. You will create the bot in Telegram in the next step.",
    ),
  ).not.toBeInTheDocument();
});

it("explains a request disappearing instead of silently starting over", async () => {
  mockGet.mockResolvedValue(configuration(request));
  const view = setup();
  await screen.findByRole("button", { name: "Get Telegram link" });
  mockGet.mockResolvedValue(configuration(null));
  await act(() =>
    view.client.invalidateQueries({ queryKey: ["telegram-new"] }),
  );
  expect(
    await screen.findByText(/This setup is no longer active/),
  ).toBeInTheDocument();
  expect(mockPost).not.toHaveBeenCalled();
});
