import "@/i18n";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { OrgGuestConversationPage } from "./org-guest-conversation-page";

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("./assistant-engine-sidebar", () => ({ AssistantEngineSidebar: () => null }));
vi.mock("./assistant-shell", () => ({ AssistantShell: ({ children }: { children: ReactNode }) => <main>{children}</main> }));

const id = `org-guest-${"a".repeat(32)}`;
const at = "2026-11-25T08:00:00Z";
const conversation = { id, agent_id: "support-agent", title: "Guest conversation", read_only: true,
  created_at: at, last_message_at: at,
  channel: { nyxbot_channel_id: "bot", platform: "lark", bot_label: "Support", chat_id: "shared",
    chat_kind: "group", chat_title: "Shared support" },
};

function show() {
  useAuthStore.setState({ user: { id: "admin" } as User, isAuthenticated: true });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}><OrgGuestConversationPage id={id} /></QueryClientProvider>);
}

afterEach(() => {
  cleanup();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null, isAuthenticated: false });
});

it("reads only the guest endpoint and shows earlier input and settled diagnostics without controls", async () => {
  const requests: string[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    expect(init.method ?? "GET").toBe("GET");
    requests.push(endpoint);
    if (endpoint.endsWith("&before_seq=2")) return Response.json({
      conversation, before_seq: null,
      messages: [{ seq: 1, role: "user", status: "completed", text: "Guest question", created_at: at }],
    });
    return Response.json({
      conversation, before_seq: 2,
      messages: [
        { seq: 2, role: "assistant", status: "completed", text: "Settled reply", created_at: at,
          tool_failures: [{ kind: "service_status", reason: "downstream_http_error", status: 502 }] },
        { seq: 3, role: "assistant", status: "failed", text: "PRIVATE_FAILED_DRAFT", created_at: at, error_code: "cancelled" },
      ],
    });
  };
  show();
  expect(await screen.findByText("Settled reply")).toBeVisible();
  expect(screen.getByText("Shared support · Lark · Support")).toBeVisible();
  expect(screen.getByText("Tool outcome: downstream_http_error · 502")).toBeVisible();
  expect(screen.getByText("Turn outcome: cancelled")).toBeVisible();
  expect(screen.queryByText("PRIVATE_FAILED_DRAFT")).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /send|retry|approve|delete|rename/i })).not.toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Load earlier messages" }));
  expect(await screen.findByText("Guest question")).toBeVisible();
  expect(requests).toEqual([
    `/assistant/nyxagent/guest-conversations/${id}?limit=50`,
    `/assistant/nyxagent/guest-conversations/${id}?limit=50&before_seq=2`,
  ]);
});

it("renders an access failure without falling back to ordinary history", async () => {
  const request = vi.fn(() => Response.json({ error: "not_found", error_code: 1003,
    message: "Guest conversation not found" }, { status: 404 }));
  globalThis.__nyxidAssistantHttpMock = request;
  show();
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the guest conversation.");
  expect(request).toHaveBeenCalledOnce();
  expect(request.mock.calls).toHaveLength(1);
});
