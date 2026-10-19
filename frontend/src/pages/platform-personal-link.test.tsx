import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PersonalLinkPage } from "./platform-personal-link";
import { PlatformPersonalBindings } from "@/components/assistant/platform-personal-bindings";
import { apiClient } from "@/lib/api-client";
import { capturePersonalLink, clearPersonalLink } from "@/lib/platform-personal-handoff";
import { useAuthStore } from "@/stores/auth-store";

vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  apiClient: vi.fn(),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { invite_code_required: true } }),
}));

const challenge = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const binding = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const token = `nyx_pli_${"a".repeat(64)}`;
let client: QueryClient;
beforeEach(() => {
  vi.mocked(apiClient).mockReset();
  clearPersonalLink();
  window.history.replaceState(null, "", `/nyxbot/link#intent_token=${token}`);
  capturePersonalLink();
  useAuthStore.setState({ user: { id: "owner", display_name: "Owner" } as never, isLoading: false });
  client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
});
afterEach(() => {
  cleanup();
  client.clear();
  clearPersonalLink();
});
function mount(children = <PersonalLinkPage />) {
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>);
}

it("uses neutral guidance while native preview metadata has not arrived", () => {
  vi.mocked(apiClient).mockImplementation(() => new Promise(() => {}));
  mount();
  expect(screen.getByRole("heading", { name: "Use your NyxBot in chat" })).toBeVisible();
  expect(screen.queryByText(/Telegram/)).not.toBeInTheDocument();
  expect(screen.queryByText(/\/login|\/link|\/confirm/)).not.toBeInTheDocument();
  expect(window.location.href).not.toContain(token);
});

it.each(["slack", "discord"] as const)("keeps %s browser review, confirmation and saved-thread continuation on its actual control surface", async (platform) => {
  const user = userEvent.setup();
  const name = platform === "discord" ? "Discord" : "Slack";
  const prefix = platform === "discord" ? "/nyx " : "";
  const chat = {
    platform, display_name: name, login_command: `${prefix}login`, logout_command: `${prefix}logout`,
    status_command: `${prefix}status`, help_command: `${prefix}help`,
    question_instruction: platform === "discord" ? 'Use /nyx ask question:"your question" in this app’s private chat.' : "Send ordinary text in this bot's private chat.",
    transport_disclosure: `This ${name} chat is operated by the platform.`,
  };
  const confirmation = platform === "discord" ? "/nyx confirm code:P-ABCD-EFGH" : "confirm P-ABCD-EFGH";
  const expires_at = new Date(Date.now() + 600000).toISOString();
  const manager = { id: "main-manager", username: null, display_name: "Main bot" };
  const sender = { id: "U-native-person", display_name: "Native Person" };
  let confirmed = false;
  vi.mocked(apiClient).mockImplementation(async (path) => {
    if (path === "/platform-personal-links/preview") return {
      chat, purpose: "personal_nyxbot", status: "pending", manager, sender, expires_at,
      invitation_required: true, requires_same_chat_confirmation: true,
    };
    if (path === "/platform-personal-links/review") return {
      chat, challenge_id: challenge, status: "review", expires_at, review_digest: "review-digest",
      account: { id: "owner", display_name: "Owner" }, permission_mode: "nyxmode", billing: "personal", replaces_existing_binding: false,
      disclosures: { telegram_transport: false, existing_nyxbot_access: true, can_approve_sensitive_actions: true, started_work_may_continue_after_unlink: true },
    };
    if (path === "/platform-personal-links/approve" || path === "/platform-personal-links/status") return {
      chat, challenge_id: challenge, status: confirmed ? "confirmed" : "awaiting_chat_confirmation", expires_at,
      binding: confirmed ? { id: binding, generation: 1 } : null,
      confirmation_command: path.endsWith("approve") ? confirmation : null,
    };
    if (path === "/platform-personal-bindings") return {
      bindings: [{ chat, id: binding, generation: 1, manager, sender, state: "active", conversation_id: "saved-native-thread", consented_at: expires_at, revoked_at: null }], next_before_id: null,
    };
    throw new Error(`Unexpected local fixture route: ${path}`);
  });
  const view = mount();
  await user.click(await screen.findByRole("button", { name: "Review personal access" }));
  expect(await screen.findByText(/Browser sign-out does not disconnect/)).toHaveTextContent(`${prefix}status`);
  expect(screen.getByText(/Browser sign-out does not disconnect/)).toHaveTextContent(`${prefix}logout`);
  expect(screen.queryByText(/Telegram/)).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Approve personal access" }));
  expect(await screen.findByText(confirmation)).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent(`same private ${name} chat`);
  confirmed = true;
  await user.click(screen.getByRole("button", { name: "Refresh status" }));
  expect(await screen.findByText(`${name} is linked.`)).toBeVisible();
  expect(screen.queryByText(confirmation)).not.toBeInTheDocument();
  view.unmount();
  mount(<PlatformPersonalBindings />);
  const saved = await screen.findByRole("link", { name: `Open saved ${name} thread` });
  expect(saved).toHaveAttribute("href", "/assistant?c=saved-native-thread");
  expect(screen.getByText(new RegExp(`${name} account`))).toHaveTextContent("Native Person");
  await waitFor(() => expect(window.location.href).not.toContain(token));
});
