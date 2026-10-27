import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { TelegramNew } from "./telegram-new";
import type { TelegramNewRequest } from "@/schemas/telegram-new";
import { ChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";

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

it("starts a tracked request without adopting an unrelated active Telegram request", async () => {
  mockGet.mockResolvedValue(configuration(request));
  mockPost.mockResolvedValue({ request, launch_url: "https://t.me/NyxSetupBot?start=challenge" });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const refresh = vi.fn().mockResolvedValue(undefined);
  render(<QueryClientProvider client={client}><ChannelConnectLinkContext.Provider value={{ token: "nyx_bcl_tracked", id: "tracked", refresh }}><TelegramNew label="Fixed name" orgId={null} fullPage onConnected={vi.fn()} /></ChannelConnectLinkContext.Provider></QueryClientProvider>);
  fireEvent.click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  await waitFor(() => expect(mockPost).toHaveBeenCalledWith("/channel-connect-links/telegram/start", { token: "nyx_bcl_tracked" }));
  expect(mockPost).toHaveBeenCalledTimes(1);
  expect(refresh).toHaveBeenCalled();
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
    expect(view.onConnected).toHaveBeenCalledExactlyOnceWith(request.id, ready.bot_username),
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
    expect(view.onConnected).toHaveBeenCalledWith(request.id, ready.bot_username),
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
    expect(view.onConnected).toHaveBeenCalledWith(request.id, ready.bot_username),
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

it("uses dialog copy for an in-place handoff", async () => {
  mockGet.mockResolvedValue(configuration({ ...request, status: "provisioning" }));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}><TelegramNew label="Support" orgId={null} stayInPlace onConnected={vi.fn()} /></QueryClientProvider>);
  expect(await screen.findByText(/You can close this dialog/)).toBeVisible();
  expect(screen.queryByText(/You can close this page/)).toBeNull();
});

const recoverable: TelegramNewRequest = {
  ...request, status: "waiting_bot", can_open_creation_form: true, expires_at: "2099-10-27T02:00:00Z",
  prompt_delivery: {
    attempt_id: "43cd44db-fc22-4f8d-aa8c-c835f8f568b2", phase: "keyboard", outcome: "rejected",
    attempted_at: "2026-10-27T01:00:00Z", completed_at: "2026-10-27T01:00:01Z",
    failure: "http_rejected", reason: "bad_request", http_status: 400, error_code: 400, retry_after: null,
  },
};
const creationUrl = "https://t.me/newbot/NyxSetupBot/support_bot?name=Support";

it.each([
  ["rejected", "Telegram rejected the creation message"],
  ["unknown", "Delivery of the Create bot message is unconfirmed"],
  ["accepted", "Telegram accepted the Create bot message"],
  ["sending", "Sending the creation instructions and button"],
] as const)("explains %s delivery without claiming rendering or creation", async (outcome, text) => {
  mockGet.mockResolvedValue(configuration({ ...recoverable, prompt_delivery: { ...recoverable.prompt_delivery!, outcome } }));
  setup();
  expect(await screen.findByText(new RegExp(text))).toHaveAttribute("role", "status");
  expect(screen.getByRole("button", { name: "Open creation form" })).toBeEnabled();
  expect(screen.queryByText(/Waiting for you to create your bot/)).not.toBeInTheDocument();
  expect(screen.queryByText(/Your bot has not been created/)).not.toBeInTheDocument();
  if (outcome === "accepted") expect(screen.getByText(/does not confirm the button appeared/)).toBeInTheDocument();
});

it("opens recovery for the exact saved request and keeps a popup fallback", async () => {
  vi.spyOn(window, "open").mockReturnValue(null);
  mockGet.mockResolvedValue(configuration(recoverable));
  mockPost.mockResolvedValue({ request: recoverable, creation_url: creationUrl });
  const view = setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  expect(await screen.findByRole("link", { name: "Open Telegram creation form" })).toHaveAttribute("href", creationUrl);
  expect(mockPost).toHaveBeenCalledExactlyOnceWith(`/channel-bots/telegram-new/requests/${request.id}/creation-link`);
  expect(view.onStarted).not.toHaveBeenCalled();
  mockGet.mockResolvedValue(configuration(ready));
  await act(() => view.client.invalidateQueries({ queryKey: ["telegram-new"] }));
  await waitFor(() => expect(screen.queryByRole("link", { name: "Open Telegram creation form" })).not.toBeInTheDocument());
});

it("refuses stale recovery when polling observes creation during issuance", async () => {
  mockGet.mockResolvedValue(configuration(recoverable));
  let settle: (value: unknown) => void = () => {};
  mockPost.mockReturnValue(new Promise((resolve) => { settle = resolve; }));
  const view = setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  expect(screen.getByRole("button", { name: "Open creation form" })).toBeDisabled();
  mockGet.mockResolvedValue(configuration(ready));
  await act(() => view.client.invalidateQueries({ queryKey: ["telegram-new"] }));
  await act(async () => settle({ request: recoverable, creation_url: creationUrl }));
  expect(replace).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
});

it("keeps the existing handoff usable when recovery is unavailable", async () => {
  mockGet.mockResolvedValue(configuration(recoverable));
  mockPost.mockRejectedValue(new Error("This setup cannot open the creation form"));
  setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  expect(await screen.findByText("This setup cannot open the creation form")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reopen Telegram" })).toBeEnabled();
  expect(close).toHaveBeenCalledOnce();
  expect(replace).not.toHaveBeenCalled();
});

it.each(["unmount", "account switch"])("discards recovery after %s", async (change) => {
  mockGet.mockResolvedValue(configuration(recoverable));
  let settle: (value: unknown) => void = () => {};
  mockPost.mockReturnValue(new Promise((resolve) => { settle = resolve; }));
  const view = setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  if (change === "unmount") view.unmount();
  else {
    mockActor.id = "actor-2";
    view.rerender(<QueryClientProvider client={view.client}><TelegramNew label="Support" orgId={null} onConnected={view.onConnected} /></QueryClientProvider>);
  }
  await act(async () => settle({ request: recoverable, creation_url: creationUrl }));
  expect(replace).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
});

it("legacy metadata absence preserves handoff and never grants recovery", async () => {
  mockGet.mockResolvedValue(configuration({ ...request, status: "waiting_bot" }));
  setup();
  expect(await screen.findByText(/Delivery of the Create bot message is unconfirmed/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Open creation form" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reopen Telegram" })).toBeEnabled();
});

it("discards a successful recovery result that arrives after its deadline", async () => {
  const expires = Date.now() + 60_000;
  const saved = { ...recoverable, expires_at: new Date(expires).toISOString() };
  mockGet.mockResolvedValue(configuration(saved));
  let settle: (value: unknown) => void = () => {};
  mockPost.mockReturnValue(new Promise((resolve) => { settle = resolve; }));
  setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  vi.spyOn(Date, "now").mockReturnValue(expires + 1);
  await act(async () => settle({ request: saved, creation_url: creationUrl }));
  expect(replace).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
  vi.restoreAllMocks();
});

it("removes the cached recovery anchor when its deadline passes with stale polling data", async () => {
  vi.spyOn(window, "open").mockReturnValue(null);
  const saved = { ...recoverable, expires_at: new Date(Date.now() + 1000).toISOString() };
  mockGet.mockResolvedValue(configuration(saved));
  mockPost.mockResolvedValue({ request: saved, creation_url: creationUrl });
  setup();
  await userEvent.setup().click(await screen.findByRole("button", { name: "Open creation form" }));
  expect(await screen.findByRole("link", { name: "Open Telegram creation form" })).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByRole("link", { name: "Open Telegram creation form" })).not.toBeInTheDocument(), { timeout: 2000 });
  expect(screen.queryByRole("button", { name: "Open creation form" })).not.toBeInTheDocument();
});
