import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { TelegramClaimPage } from "./telegram-claim";
import {
  captureTelegramClaim,
  clearTelegramClaimHandoff,
  preserveTelegramClaimForLogin,
} from "@/lib/telegram-claim-handoff";
import { ApiError } from "@/lib/api-client";
const { get, post, remove, navigate, actor } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  remove: vi.fn(),
  navigate: vi.fn(),
  actor: { id: "actor-1" as string | undefined },
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get, post, delete: remove },
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (
    selector: (state: { user: { id: string | undefined } }) => unknown,
  ) => selector({ user: actor }),
}));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
    disabled,
  }: {
    value: string | null;
    onChange: (next: string | null) => void;
    disabled: boolean;
  }) => (
    <select
      aria-label="Connect to"
      value={value ?? ""}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value || null)}
    >
      <option value="">Personal</option>
      <option value="cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65">Support team</option>
    </select>
  ),
}));
const code = "ABCDE-FGHJK-LMNPQ-RSTUV";
const root = "/channel-bots/telegram-new";
const request = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
  status: "ready",
  revision: 0,
  label: "Support",
  owner_user_id: "actor-1",
  auto_connect: true,
  expires_at: "2026-09-15T10:00:00Z",
  telegram_bot_id: "900",
  bot_username: "CustomerBot",
  channel_bot_id: null,
};
function error(status: number, message: string) {
  return new ApiError(status, { error: "test_error", message, error_code: status });
}
function preview() {
  return {
    bot_username: "CustomerBot",
    expires_at: new Date(Date.now() + 10 * 60_000).toISOString(),
  };
}
function setup(withCode = true) {
  if (withCode) {
    window.history.replaceState(
      null,
      "",
      `/channel-bots?connect=telegram-new&claim=${code}`,
    );
    captureTelegramClaim();
    preserveTelegramClaimForLogin();
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = () => (
    <StrictMode>
      <QueryClientProvider client={client}>
        <TelegramClaimPage />
      </QueryClientProvider>
    </StrictMode>
  );
  return { ...render(view()), client, view };
}
beforeEach(() => {
  vi.resetAllMocks();
  actor.id = "actor-1";
  clearTelegramClaimHandoff();
  window.history.replaceState(
    null,
    "",
    "/channel-bots?connect=telegram-new&claim_entry=true",
  );
  post.mockImplementation(async (url: string) =>
    url.endsWith("preview") ? preview() : request,
  );
});
it("previews once after login, connects only on click, and keeps the code out of cache and storage", async () => {
  const user = userEvent.setup();
  const view = setup();
  expect(
    await screen.findByRole("heading", { name: "Connect @CustomerBot" }),
  ).toBeVisible();
  expect(post).toHaveBeenCalledExactlyOnceWith(`${root}/claims/preview`, {
    code,
  });
  expect(get).not.toHaveBeenCalled();
  expect(JSON.stringify({ localStorage, sessionStorage })).not.toContain(code);
  await user.clear(screen.getByLabelText("Bot label in NyxID"));
  await user.type(screen.getByLabelText("Bot label in NyxID"), "Support");
  await user.selectOptions(
    screen.getByLabelText("Connect to"),
    "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65",
  );
  await user.click(
    screen.getByRole("button", { name: "Connect" }),
  );
  await waitFor(() =>
    expect(navigate).toHaveBeenCalledWith({
      to: "/channel-bots",
      search: { connect: "telegram-new", request_id: request.id },
      replace: true,
    }),
  );
  expect(post).toHaveBeenLastCalledWith(`${root}/claims/redeem`, {
    code,
    label: "Support",
    target_org_id: "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65",
  });
  expect(
    JSON.stringify(
      view.client
        .getMutationCache()
        .getAll()
        .map((m) => m.state),
    ),
  ).not.toContain(code);
  expect(JSON.stringify(view.client.getQueryCache().getAll())).not.toContain(
    code,
  );
});
it("lets a user manually enter a code without requests on mount", async () => {
  setup(false);
  expect(post).not.toHaveBeenCalled();
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Claim code"), code);
  await user.click(screen.getByRole("button", { name: "Check code" }));
  expect(
    await screen.findByRole("button", { name: "Connect" }),
  ).toBeVisible();
  expect(post).toHaveBeenCalledTimes(1);
});
it("keeps the code for explicit retry while Telegram confirmation is pending", async () => {
  post.mockRejectedValueOnce(
    error(409, "Telegram is still confirming creation. Try again in a moment."),
  );
  setup();
  expect(await screen.findByText(/Telegram is still confirming/)).toBeVisible();
  expect(screen.getByLabelText("Claim code")).toHaveValue(code);
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByRole("button", { name: "Connect" }),
  ).toBeVisible();
  expect(post).toHaveBeenCalledTimes(2);
});
it("leaves a conflicting setup intact until the user explicitly cancels it", async () => {
  post.mockImplementation(async (url: string) => {
    if (url.endsWith("preview")) return preview();
    throw error(
      409,
      "Finish or cancel your existing Telegram creation request first",
    );
  });
  get.mockResolvedValue({
    available: true,
    manager_username: "ManagerBot",
    request: { ...request, status: "waiting_bot" },
  });
  setup();
  const user = userEvent.setup();
  await user.click(
    await screen.findByRole("button", { name: "Connect" }),
  );
  expect(
    await screen.findByRole("button", { name: "Cancel setup" }),
  ).toBeVisible();
  expect(remove).not.toHaveBeenCalled();
  get.mockResolvedValue({
    available: true,
    manager_username: "ManagerBot",
    request: null,
  });
  remove.mockResolvedValue(undefined);
  await user.click(screen.getByRole("button", { name: "Cancel setup" }));
  expect(remove).toHaveBeenCalledWith(`${root}/requests/${request.id}`);
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Cancel setup" }),
    ).not.toBeInTheDocument(),
  );
  expect(
    screen.getByRole("button", { name: "Connect" }),
  ).toBeEnabled();
});
it("pins the draft for a retry after an uncertain redemption response", async () => {
  post.mockImplementation(async (url: string) => {
    if (url.endsWith("preview")) return preview();
    throw new Error("Connection interrupted");
  });
  setup();
  const user = userEvent.setup();
  await user.click(
    await screen.findByRole("button", { name: "Connect" }),
  );
  expect(
    await screen.findByRole("button", { name: "Retry connection" }),
  ).toBeVisible();
  expect(screen.getByLabelText("Bot label in NyxID")).toBeDisabled();
  expect(screen.getByLabelText("Connect to")).toBeDisabled();
  post.mockResolvedValue(request);
  await user.click(screen.getByRole("button", { name: "Retry connection" }));
  await waitFor(() => expect(navigate).toHaveBeenCalled());
});
it("clears expired codes and asks for a fresh code", async () => {
  post.mockRejectedValue(error(404, "Telegram claim not found or expired"));
  setup();
  expect(
    await screen.findByText("Telegram claim not found or expired"),
  ).toBeVisible();
  expect(screen.getByLabelText("Claim code")).toHaveValue("");
  expect(JSON.stringify(sessionStorage)).not.toContain(code);
});
it("discards the claim on account change and ignores an outstanding preview", async () => {
  let resolve: (value: unknown) => void = () => {};
  post.mockReturnValue(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const view = setup();
  await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
  actor.id = "actor-2";
  view.rerender(view.view());
  await act(async () => resolve(preview()));
  expect(screen.getByLabelText("Claim code")).toHaveValue("");
  expect(
    screen.queryByRole("heading", { name: "Connect @CustomerBot" }),
  ).not.toBeInTheDocument();
  expect(post).toHaveBeenCalledTimes(1);
  view.unmount();
  setup(false);
  expect(screen.getByLabelText("Claim code")).toHaveValue("");
});

it("waits for the signed-in user before consuming the login handoff", async () => {
  actor.id = undefined;
  const view = setup();
  expect(screen.getByRole("status")).toHaveTextContent("Loading your account");
  expect(JSON.stringify(sessionStorage)).toContain(code);
  expect(post).not.toHaveBeenCalled();
  actor.id = "actor-1";
  view.rerender(view.view());
  expect(
    await screen.findByRole("heading", { name: "Connect @CustomerBot" }),
  ).toBeVisible();
  expect(JSON.stringify(sessionStorage)).not.toContain(code);
  expect(post).toHaveBeenCalledExactlyOnceWith(`${root}/claims/preview`, {
    code,
  });
});
