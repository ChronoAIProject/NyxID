import { ComposerSettings } from "./composer-settings";
import { modelDisplayName } from "@/lib/assistant/nyxbot-labels";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
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
    capabilities: { billing_available: false, enabled_features: ["assistant:model-controls"] },
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
  return userEvent.setup({ skipHover: true });
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
  expect(select).toHaveTextContent("Default model");
  await user.click(select);
  await user.click(await screen.findByRole("option", { name: "Research" }));
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
  await user.click(await screen.findByRole("option", { name: "Research" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Choose another model in the model selector");
});

it("shows and switches the workspace access mode from a thread", async () => {
  const user = renderWith(<AccessModeSwitcher />);
  const select = await screen.findByRole("combobox", { name: "Access mode" });
  expect(select).toHaveTextContent("Auto");
  await user.click(select);
  await user.click(await screen.findByRole("option", { name: "Ask every time" }));
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
  expect(screen.queryByRole("combobox", { name: "Access mode" })).not.toBeInTheDocument();
});

function useExternalUser() {
  const user = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({ ...user, capabilities: { billing_available: false, enabled_features: [] } });
}

it("shows external users only Thinking and Permissions and maps Thinking to research", async () => {
  useExternalUser();
  const select = vi.fn();
  const user = renderWith(<ComposerSettings model="nyxagent/chat" onDraftModelChange={select} />);
  await user.click(await screen.findByRole("button", { name: /Thinking and permissions: Auto/ }));
  expect(screen.queryByRole("menuitem", { name: /^(Model|Reasoning)/ })).not.toBeInTheDocument();
  const toggle = screen.getByRole("menuitemcheckbox", { name: "Thinking mode" });
  expect(toggle).toHaveAttribute("aria-checked", "false");
  await user.click(toggle);
  expect(select).toHaveBeenCalledWith("nyxagent/research");
  expect(writes).toEqual([]);
});

it("turns Thinking off by selecting chat for an existing external thread", async () => {
  useExternalUser();
  const user = renderWith(<ComposerSettings conversationId={thread} model="nyxagent/research" onDraftModelChange={() => {}} />);
  await user.click(await screen.findByRole("button", { name: /Thinking and permissions: Auto, Thinking/ }));
  await user.click(screen.getByRole("menuitemcheckbox", { name: "Thinking mode" }));
  await waitFor(() => expect(writes).toEqual([
    { method: "PATCH", endpoint: `/assistant/nyxagent/conversations/${thread}/model`, body: { model: "nyxagent/chat" } },
  ]));
});

it("disables Thinking during an active turn", async () => {
  useExternalUser();
  const user = renderWith(<ComposerSettings model="nyxagent/chat" active onDraftModelChange={() => {}} />);
  await user.click(await screen.findByRole("button", { name: /Thinking and permissions: Auto/ }));
  expect(screen.getByRole("menuitemcheckbox", { name: "Thinking mode" })).toHaveAttribute("aria-disabled", "true");
});

it("combines concise permissions and models in one menu and saves the selected permission", async () => {
  const user = renderWith(<ComposerSettings conversationId={thread} model="nyxagent/chat" onDraftModelChange={() => {}} />);
  const trigger = await screen.findByRole("button", { name: "Model and permissions: Default model, Auto" });
  await user.click(trigger);
  expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitemcheckbox", { name: "Thinking mode" })).not.toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: /Reasoning/ })).toHaveAttribute("aria-disabled", "true");
  await user.click(screen.getByRole("menuitem", { name: /^Permissions/ }));
  expect(screen.getAllByRole("menuitemradio")).toHaveLength(3);
  expect(screen.getByRole("menuitemradio", { name: "Auto" })).toHaveAttribute("aria-checked", "true");
  expect(screen.queryByText(/Acts on ordinary/)).not.toBeInTheDocument();
  await user.click(screen.getByRole("menuitemradio", { name: "Full access" }));
  await waitFor(() => expect(writes).toEqual([
    { method: "PUT", endpoint: "/assistant/nyxagent/settings", body: { access_mode: "autopilot" } },
  ]));
  expect(await screen.findByRole("button", { name: /Model and permissions: Default model, Full access/ })).toBeVisible();
});

it("changes an existing thread's model from the combined menu", async () => {
  const user = renderWith(<ComposerSettings conversationId={thread} model="nyxagent/chat" onDraftModelChange={() => {}} />);
  await user.click(await screen.findByRole("button", { name: /Model and permissions: Default model, Auto/ }));
  await user.click(screen.getByRole("menuitem", { name: /^Model / }));
  await user.click(await screen.findByRole("menuitemradio", { name: "Research" }));
  await waitFor(() => expect(writes).toEqual([
    { method: "PATCH", endpoint: `/assistant/nyxagent/conversations/${thread}/model`, body: { model: "nyxagent/research" } },
  ]));
});

it("holds a new thread's selection locally without changing an agent or another thread", async () => {
  const select = vi.fn();
  const user = renderWith(<ComposerSettings model="nyxagent/chat" onDraftModelChange={select} />);
  await user.click(await screen.findByRole("button", { name: /Model and permissions: Default model, Auto/ }));
  await user.click(screen.getByRole("menuitem", { name: /^Model / }));
  await user.click(await screen.findByRole("menuitemradio", { name: "Research" }));
  expect(select).toHaveBeenCalledWith("nyxagent/research");
  expect(writes).toEqual([]);
});

it("locks model changes during a turn and preserves an agent's permission override", async () => {
  const user = renderWith(<ComposerSettings conversationId={thread} model="nyxagent/chat" active
    agentOverride="ask_every_time" onDraftModelChange={() => {}} />);
  await user.click(await screen.findByRole("button", { name: /Model and permissions: Default model, Ask every time/ }));
  await user.click(screen.getByRole("menuitem", { name: /^Model / }));
  for (const item of screen.getAllByRole("menuitemradio")) expect(item).toHaveAttribute("aria-disabled", "true");
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: /Model and permissions:/ }));
  await user.click(screen.getByRole("menuitem", { name: /^Permissions/ }));
  for (const item of screen.getAllByRole("menuitemradio")) expect(item).toHaveAttribute("aria-disabled", "true");
  expect(screen.getByRole("menuitemradio", { name: "Ask every time" })).toHaveAttribute("aria-checked", "true");
  expect(writes).toEqual([]);
});

it("uses actual upstream display names without rewriting them", () => {
  expect(modelDisplayName({ id: "nyxagent/chat", label: "Claude Sonnet 4.5" })).toBe("Claude Sonnet 4.5");
  expect(modelDisplayName({ id: "nyxagent/chat", label: "chat" })).toBe("Default model");
});

it("disables Thinking when NyxAgent does not offer research", async () => {
  useExternalUser();
  const select = vi.fn();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint === "/assistant/nyxagent/settings") return json(settings);
    if (endpoint === "/assistant/nyxagent/models") return json([
      { id: "nyxagent/chat", label: "chat", available: true },
      { id: "nyxagent/research", label: "research", available: false },
    ]);
    return json({}, 404);
  };
  const user = renderWith(<ComposerSettings model="nyxagent/chat" onDraftModelChange={select} />);
  await user.click(await screen.findByRole("button", { name: /Thinking and permissions: Auto/ }));
  const toggle = screen.getByRole("menuitemcheckbox", { name: "Thinking mode" });
  expect(toggle).toHaveAttribute("aria-disabled", "true");
  await user.click(toggle);
  expect(select).not.toHaveBeenCalled();
});
