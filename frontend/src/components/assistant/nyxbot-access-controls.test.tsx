import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import {
  AccessModeSwitcher,
  ThreadModelSelect,
  modelOptions,
} from "./nyxbot-access-controls";

const at = "2026-10-12T00:00:00Z";
const thread = `nyxa-${"a".repeat(32)}`;
let writes: { method: string; endpoint: string; body: unknown }[];
let settings: Record<string, unknown>;
let modelStatus: number;
let client: QueryClient;

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });

beforeEach(() => {
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: at,
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  writes = [];
  modelStatus = 200;
  settings = {
    access_mode: "nyxmode",
    skip_destructive_confirmation: false,
    max_live_subagents: 8,
    max_concurrent_subagent_turns: 3,
    max_live_subagents_limit: 32,
    max_concurrent_subagent_turns_limit: 8,
  };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    if (method !== "GET") writes.push({ method, endpoint, body });
    if (endpoint === "/assistant/nyxagent/settings") {
      if (method === "PUT") settings = { ...settings, ...(body as object) };
      return json(settings);
    }
    if (endpoint === "/assistant/nyxagent/models") {
      return json([
        { id: "nyxagent/chat", label: "chat", available: true },
        { id: "nyxagent/research", label: "research", available: true },
      ]);
    }
    if (endpoint === `/assistant/nyxagent/conversations/${thread}/model`) {
      if (modelStatus !== 200) {
        return json(
          {
            error: "model_unavailable",
            error_code: 12103,
            message: "The selected model is not available. Choose another model in the model selector.",
          },
          modelStatus,
        );
      }
      return json({
        id: thread,
        title: "Thread",
        model: (body as { model: string }).model,
        created_at: at,
        last_message_at: at,
        message_count: 1,
        active_turn: null,
        context_reset_at: at,
        context_reset_reason: "model_changed",
      });
    }
    return json({ message: "not found" }, 404);
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

function renderWith(node: React.ReactNode) {
  render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
  return userEvent.setup();
}

it("keeps a model NyxAgent no longer offers visible as unavailable", () => {
  const rows = modelOptions([{ id: "nyxagent/chat", label: "chat", available: true }], "nyxagent/old");
  expect(rows).toEqual([
    { id: "nyxagent/old", label: "old", available: false },
    { id: "nyxagent/chat", label: "chat", available: true },
  ]);
  expect(modelOptions(rows, "nyxagent/chat")).toHaveLength(2);
});

it("switches a thread's model through the server", async () => {
  const user = renderWith(<ThreadModelSelect conversationId={thread} model="nyxagent/chat" />);
  const select = await screen.findByRole("combobox", { name: "Model" });
  await waitFor(() => expect(select).toBeEnabled());
  expect(select).toHaveTextContent("chat");
  await user.click(select);
  await user.click(await screen.findByRole("option", { name: "research" }));
  await waitFor(() =>
    expect(writes).toEqual([
      {
        method: "PATCH",
        endpoint: `/assistant/nyxagent/conversations/${thread}/model`,
        body: { model: "nyxagent/research" },
      },
    ]),
  );
});

it("shows the server's actionable refusal of a model", async () => {
  modelStatus = 400;
  const user = renderWith(<ThreadModelSelect conversationId={thread} model="nyxagent/chat" />);
  const select = await screen.findByRole("combobox", { name: "Model" });
  await waitFor(() => expect(select).toBeEnabled());
  await user.click(select);
  await user.click(await screen.findByRole("option", { name: "research" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Choose another model in the model selector");
});

it("shows and switches the workspace access mode from a thread", async () => {
  const user = renderWith(<AccessModeSwitcher />);
  const select = await screen.findByRole("combobox", { name: "Permission mode" });
  expect(select).toHaveTextContent("Auto");
  await user.click(select);
  await user.click(await screen.findByRole("option", { name: /^Ask every time/ }));
  await waitFor(() =>
    expect(writes).toEqual([
      { method: "PUT", endpoint: "/assistant/nyxagent/settings", body: { access_mode: "ask_every_time" } },
    ]),
  );
  await waitFor(() => expect(select).toHaveTextContent("Ask every time"));
});

it("shows an agent's own access mode instead of the switcher", async () => {
  renderWith(<AccessModeSwitcher agentOverride="autopilot" />);
  expect(await screen.findByLabelText("Access mode: Full access (this agent)")).toBeVisible();
  expect(screen.queryByRole("combobox", { name: "Permission mode" })).not.toBeInTheDocument();
});
