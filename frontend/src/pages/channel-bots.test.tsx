import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { ChannelBotsPage } from "./channel-bots";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

const { get, post, remove } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  remove: vi.fn(),
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get, post, delete: remove },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "actor-1" } }),
}));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
    disabled,
    label = "Scope",
  }: {
    value: string | null;
    onChange: (next: string | null) => void;
    disabled?: boolean;
    label?: string;
  }) => (
    <select
      aria-label={label}
      value={value ?? ""}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value || null)}
    >
      <option value="">Personal</option>
      <option value="cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65">Support team</option>
    </select>
  ),
}));

const root = "/channel-bots/telegram-new";
const orgId = "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65";
const request: TelegramNewRequest = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
  status: "waiting_telegram",
  revision: 1,
  owner_user_id: orgId,
  label: "Saved support",
  expires_at: "2026-09-16T10:00:00Z",
  telegram_bot_id: null,
  bot_username: null,
  channel_bot_id: null,
  auto_connect: true,
};
let saved: TelegramNewRequest | null;
let available: boolean;

async function setup(url = "/channel-bots?connect=telegram-new") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const rootRoute = createRootRoute();
  const list = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channel-bots",
    component: ChannelBotsPage,
    validateSearch: (search) => search,
  });
  const detail = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channel-bots/$botId",
    component: () => <h1>Bot details</h1>,
  });
  const history = createMemoryHistory({ initialEntries: [url] });
  const router = createRouter({
    routeTree: rootRoute.addChildren([list, detail]),
    history,
  });
  const view = render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  );
  await act(() => router.load());
  return { ...view, client, router, history };
}

beforeEach(() => {
  vi.resetAllMocks();
  saved = null;
  available = true;
  get.mockImplementation(async (path: string) => {
    if (path.startsWith(root))
      return { available, manager_username: "NyxSetupBot", request: saved };
    if (path.startsWith("/channel-bots")) return { bots: [], total: 0 };
    if (path.startsWith("/channel-conversations"))
      return { conversations: [], total: 0 };
    if (path === "/api-keys") return { keys: [] };
    throw new Error(`Unexpected GET ${path}`);
  });
  vi.spyOn(window, "open").mockReturnValue(null);
});

it("persists a draft from the shared modal fields with replacement navigation and restores it on remount", async () => {
  const user = userEvent.setup();
  const view = await setup();
  const dialog = within(await screen.findByRole("dialog"));
  const label = dialog.getByLabelText("Label", { exact: true });
  await waitFor(() => expect(label).toBeEnabled());
  await user.type(label, "Draft support");
  await user.selectOptions(
    dialog.getByRole("combobox", { name: "Scope" }),
    orgId,
  );
  await waitFor(() =>
    expect(view.router.state.location.search).toEqual({
      connect: "telegram-new",
      label: "Draft support",
      target_org_id: orgId,
    }),
  );
  expect(view.history.length).toBe(1);
  const url = view.router.state.location.href;
  view.unmount();
  view.client.clear();
  await setup(url);
  const resumed = within(await screen.findByRole("dialog"));
  expect(resumed.getByLabelText("Label", { exact: true })).toHaveValue(
    "Draft support",
  );
  expect(resumed.getByRole("combobox", { name: "Scope" })).toHaveValue(orgId);
  expect(
    screen.getByRole("heading", {
      name: "Channel Bots",
      hidden: true,
    }),
  ).toBeInTheDocument();
  expect(
    get.mock.calls.some(([path]) => path.includes("managed-onboarding")),
  ).toBe(false);
});

it.each([orgId, "actor-1"])(
  "restores request_id and locks the server-saved label and owner %s over URL drafts",
  async (owner) => {
    saved = { ...request, owner_user_id: owner };
    const view = await setup(
      `/channel-bots?connect=telegram-new&label=Stale&target_org_id=${orgId}&request_id=${request.id}`,
    );
    const dialog = within(await screen.findByRole("dialog"));
    await waitFor(() =>
      expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
        request.label,
      ),
    );
    expect(dialog.getByLabelText("Label", { exact: true })).toBeDisabled();
    expect(dialog.getByRole("combobox", { name: "Scope" })).toHaveValue(
      owner === orgId ? orgId : "",
    );
    expect(dialog.getByRole("combobox", { name: "Scope" })).toBeDisabled();
    expect(
      dialog.getByText(/To change them, choose Cancel setup/),
    ).toBeVisible();
    expect(get).toHaveBeenCalledWith(`${root}?request_id=${request.id}`);
    await waitFor(() =>
      expect(view.router.state.location.search).toEqual({
        connect: "telegram-new",
        label: request.label,
        target_org_id: owner === orgId ? orgId : undefined,
        request_id: request.id,
      }),
    );
    expect(post).not.toHaveBeenCalled();
  },
);

it("records a started request without putting its launch challenge in the URL", async () => {
  post.mockImplementation(async () => {
    saved = request;
    return {
      request,
      launch_url: "https://t.me/NyxSetupBot?start=private-challenge",
    };
  });
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&label=Saved%20support&target_org_id=${orgId}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(
    await dialog.findByRole("button", { name: "Continue in Telegram" }),
  );
  await waitFor(() =>
    expect(view.router.state.location.search.request_id).toBe(request.id),
  );
  expect(post).toHaveBeenCalledExactlyOnceWith(root, {
    label: request.label,
    target_org_id: orgId,
    auto_connect: true,
  });
  expect(view.router.state.location.href).not.toContain("challenge");
  expect(dialog.getByLabelText("Label", { exact: true })).toBeDisabled();
});

it("dismisses without cancelling or reopening, then resumes through a later search change", async () => {
  saved = request;
  const user = userEvent.setup();
  const view = await setup();
  await screen.findByText(/To change them, choose Cancel setup/);
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(view.router.state.location.search.connect).toBeUndefined();
  expect(view.router.state.location.search.request_id).toBe(request.id);
  await act(() =>
    view.client.invalidateQueries({ queryKey: ["telegram-new"] }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(remove).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "Resume Telegram setup" }),
  );
  const dialog = within(await screen.findByRole("dialog"));
  await waitFor(() =>
    expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
      request.label,
    ),
  );
  expect(remove).not.toHaveBeenCalled();
  expect(post).not.toHaveBeenCalled();
});

it("only Cancel setup cancels the request, removes request_id, and unlocks the shared fields", async () => {
  saved = request;
  remove.mockImplementation(async () => {
    saved = null;
  });
  const user = userEvent.setup();
  const view = await setup(
    `/channel-bots?connect=telegram-new&request_id=${request.id}`,
  );
  const dialog = within(await screen.findByRole("dialog"));
  await user.click(await dialog.findByRole("button", { name: "Cancel setup" }));
  await waitFor(() =>
    expect(dialog.getByLabelText("Label", { exact: true })).toBeEnabled(),
  );
  expect(dialog.getByRole("combobox", { name: "Scope" })).toBeEnabled();
  expect(dialog.getByLabelText("Label", { exact: true })).toHaveValue(
    request.label,
  );
  await waitFor(() =>
    expect(view.router.state.location.search.request_id).toBeUndefined(),
  );
  expect(view.router.state.location.search.connect).toBe("telegram-new");
  expect(remove).toHaveBeenCalledExactlyOnceWith(
    `${root}/requests/${request.id}`,
  );
});

it("follows a completed request_id to bot details and closes the modal", async () => {
  saved = {
    ...request,
    status: "connected",
    channel_bot_id: "connected-bot",
    bot_username: "SupportBot",
    telegram_bot_id: "900",
  };
  const view = await setup(
    `/channel-bots?connect=telegram-new&request_id=${request.id}`,
  );
  await screen.findByRole("heading", { name: "Bot details" });
  expect(view.router.state.location.pathname).toBe(
    "/channel-bots/connected-bot",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(get).toHaveBeenCalledWith(`${root}?request_id=${request.id}`);
  expect(post).not.toHaveBeenCalled();
});

it("shows the manager configuration message inside the modal without managed onboarding", async () => {
  available = false;
  await setup();
  const dialog = within(await screen.findByRole("dialog"));
  expect(
    await dialog.findByText(
      /An administrator needs to configure a manager bot/,
    ),
  ).toBeVisible();
  expect(
    get.mock.calls.some(([path]) => path.includes("managed-onboarding")),
  ).toBe(false);
  expect(dialog.queryByLabelText("Bot Token")).not.toBeInTheDocument();
});
