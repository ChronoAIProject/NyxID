import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { TelegramNew } from "./telegram-new";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

const { mockGet, mockPost, mockDelete, mockActor } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockDelete: vi.fn(),
  mockActor: { id: "actor-1" },
}));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, delete: mockDelete },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: mockActor }),
}));

const request: TelegramNewRequest = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
  status: "waiting_telegram",
  revision: 1,
  owner_user_id: "actor-1",
  label: "Support",
  expires_at: "2026-09-15T10:00:00Z",
  telegram_bot_id: null,
  bot_username: null,
  channel_bot_id: null,
  auto_connect: true,
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
const replace = vi.fn();
const close = vi.fn();

function setup(requestId?: string) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const onConnected = vi.fn();
  const onStarted = vi.fn();
  const onCancelled = vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <TelegramNew
        label="Support"
        orgId={null}
        requestId={requestId}
        onStarted={onStarted}
        onCancelled={onCancelled}
        onConnected={onConnected}
      />
    </QueryClientProvider>,
  );
  return { ...view, client, onConnected, onStarted, onCancelled };
}

beforeEach(() => {
  vi.resetAllMocks();
  mockActor.id = "actor-1";
  vi.spyOn(window, "open").mockReturnValue({
    opener: null,
    closed: false,
    location: { replace },
    close,
  } as unknown as Window);
});

it("saves and opens Telegram with one click, then completes from server status without a Connect request", async () => {
  const user = userEvent.setup();
  mockGet.mockResolvedValue(configuration(null));
  mockPost.mockImplementation(async () => {
    mockGet.mockResolvedValue(configuration(request));
    return { request, launch_url: "https://t.me/NyxSetupBot?start=challenge" };
  });
  const view = setup();
  await user.click(
    await screen.findByRole("button", { name: "Continue in Telegram" }),
  );
  await waitFor(() =>
    expect(replace).toHaveBeenCalledWith(
      "https://t.me/NyxSetupBot?start=challenge",
    ),
  );
  expect(mockPost).toHaveBeenCalledExactlyOnceWith(
    "/channel-bots/telegram-new",
    { label: "Support", auto_connect: true },
  );
  expect(view.onStarted).toHaveBeenCalledWith(request.id);
  expect(
    within(
      screen.getByRole("list", { name: "Telegram setup steps" }),
    ).getAllByRole("listitem"),
  ).toHaveLength(2);
  expect(
    screen.queryByRole("button", { name: "Connect bot" }),
  ).not.toBeInTheDocument();
  mockGet.mockResolvedValue(
    configuration({
      ...ready,
      status: "connected",
      channel_bot_id: request.id,
    }),
  );
  await act(() =>
    view.client.invalidateQueries({ queryKey: ["telegram-new"] }),
  );
  await waitFor(() =>
    expect(view.onConnected).toHaveBeenCalledExactlyOnceWith(request.id),
  );
  expect(mockPost).toHaveBeenCalledTimes(1);
});

it("shows a fallback link when a browser blocks the Telegram tab", async () => {
  vi.mocked(window.open).mockReturnValue(null);
  mockGet.mockResolvedValue(configuration(null));
  mockPost.mockImplementation(async () => {
    mockGet.mockResolvedValue(configuration(request));
    return { request, launch_url: "https://t.me/NyxSetupBot?start=challenge" };
  });
  setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  expect(
    await screen.findByRole("link", { name: "Open Telegram" }),
  ).toHaveAttribute("href", "https://t.me/NyxSetupBot?start=challenge");
});

it("reads the saved request after reload even when it is already completed", async () => {
  mockGet.mockResolvedValue(
    configuration({
      ...ready,
      status: "connected",
      channel_bot_id: request.id,
    }),
  );
  const view = setup(request.id);
  await waitFor(() =>
    expect(view.onConnected).toHaveBeenCalledWith(request.id),
  );
  expect(mockGet).toHaveBeenCalledWith(
    `/channel-bots/telegram-new?request_id=${request.id}`,
  );
  expect(mockPost).not.toHaveBeenCalled();
});

it("leaves interrupted provisioning to server retries without asking for another connection click", async () => {
  mockGet.mockResolvedValue(
    configuration({
      ...ready,
      status: "provisioning",
      channel_bot_id: request.id,
      connection_error:
        "Telegram setup is taking longer than usual. We are retrying automatically.",
    }),
  );
  setup();
  expect(
    await screen.findByText("Connecting @SupportBot…"),
  ).toBeInTheDocument();
  expect(screen.getByText(/retrying automatically/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", {
      name: /Connect bot|Retry connection|Finish saved/,
    }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Cancel setup" }),
  ).not.toBeInTheDocument();
  expect(mockPost).not.toHaveBeenCalled();
});

it("reopens the existing handoff without starting a second creation", async () => {
  mockGet.mockResolvedValue(configuration(request));
  mockPost.mockResolvedValue({
    request,
    launch_url: "https://t.me/NyxSetupBot?start=fresh",
  });
  setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Reopen Telegram" }));
  await waitFor(() =>
    expect(replace).toHaveBeenCalledWith(
      "https://t.me/NyxSetupBot?start=fresh",
    ),
  );
  expect(mockPost).toHaveBeenCalledExactlyOnceWith(
    `/channel-bots/telegram-new/requests/${request.id}/launch`,
  );
});

it("closes the blank tab on a failed start and shows the error", async () => {
  mockGet.mockResolvedValue(configuration(null));
  mockPost.mockRejectedValue(new Error("Setup unavailable"));
  setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  expect(await screen.findByText("Setup unavailable")).toBeInTheDocument();
  expect(close).toHaveBeenCalledOnce();
  expect(replace).not.toHaveBeenCalled();
});

it("does not launch a completed response after the setup page was closed", async () => {
  mockGet.mockResolvedValue(configuration(null));
  let resolveStart: (value: unknown) => void = () => {};
  mockPost.mockReturnValue(
    new Promise((resolve) => {
      resolveStart = resolve;
    }),
  );
  const view = setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  view.unmount();
  await act(async () =>
    resolveStart({
      request,
      launch_url: "https://t.me/NyxSetupBot?start=challenge",
    }),
  );
  expect(replace).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
});

it("discards an outstanding handoff when the signed-in account changes", async () => {
  mockGet.mockResolvedValue(configuration(null));
  let resolveStart: (value: unknown) => void = () => {};
  mockPost.mockReturnValue(
    new Promise((resolve) => {
      resolveStart = resolve;
    }),
  );
  const view = setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  mockActor.id = "actor-2";
  view.rerender(
    <QueryClientProvider client={view.client}>
      <TelegramNew
        label="Support"
        orgId={null}
        onStarted={view.onStarted}
        onConnected={view.onConnected}
      />
    </QueryClientProvider>,
  );
  await act(async () =>
    resolveStart({
      request,
      launch_url: "https://t.me/NyxSetupBot?start=challenge",
    }),
  );
  expect(replace).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
  expect(view.onStarted).not.toHaveBeenCalled();
});

it("cancels an unprovisioned request and clears its saved page reference", async () => {
  mockGet.mockResolvedValue(configuration(request));
  mockDelete.mockImplementation(async () => {
    mockGet.mockResolvedValue(configuration(null));
  });
  const view = setup();
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Cancel setup" }));
  await waitFor(() => expect(view.onCancelled).toHaveBeenCalledOnce());
  expect(mockDelete).toHaveBeenCalledWith(
    `/channel-bots/telegram-new/requests/${request.id}`,
  );
});

it("can finish a request created by the previous explicit approval flow", async () => {
  mockGet.mockResolvedValue(configuration({ ...ready, auto_connect: false }));
  mockPost.mockResolvedValue({
    ...ready,
    auto_connect: false,
    status: "connected",
    channel_bot_id: request.id,
  });
  const view = setup();
  await userEvent
    .setup()
    .click(
      await screen.findByRole("button", { name: "Finish saved connection" }),
    );
  await waitFor(() =>
    expect(view.onConnected).toHaveBeenCalledWith(request.id),
  );
  expect(mockPost).toHaveBeenCalledWith(
    `/channel-bots/telegram-new/requests/${request.id}/connect`,
    { telegram_bot_id: "900", revision: 6 },
  );
});

it("does not offer creation until a manager is configured", async () => {
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
    screen.queryByRole("button", { name: "Continue in Telegram" }),
  ).not.toBeInTheDocument();
});
