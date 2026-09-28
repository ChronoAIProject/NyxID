import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { NewAgentDialog } from "./nyxbot-agent-forms";

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    isPending: false,
    data: [
      { slug: "github", label: "GitHub", is_active: true },
      { slug: "slack", label: "Slack", is_active: true },
      { slug: "old-api", label: "Old API", is_active: false },
    ],
  }),
}));

const home = `nyxa-${"d".repeat(32)}`;
let posts: unknown[];
let respond: () => Response;
let client: QueryClient;

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
    created_at: "2026-09-28T00:00:00Z",
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  posts = [];
  respond = () =>
    new Response(JSON.stringify({ id: "agent-new", name: "researcher", home_conversation_id: home }), {
      status: 201,
    });
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint === "/assistant/nyxagent/agents" && init.method === "POST") {
      posts.push(JSON.parse(String(init.body)));
      return respond();
    }
    return new Response(JSON.stringify({ agents: [], limits: {} }));
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

function renderDialog() {
  const onCreated = vi.fn();
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <NewAgentDialog onClose={onClose} onCreated={onCreated} />
    </QueryClientProvider>,
  );
  return { onCreated, onClose, user: userEvent.setup() };
}

it("creates a specialist with its role, services and account read, then opens its home thread", async () => {
  const { onCreated, user } = renderDialog();
  const dialog = screen.getByRole("dialog", { name: "New agent" });
  const create = within(dialog).getByRole("button", { name: "Create agent" });
  expect(create).toBeDisabled();
  // Only connected, active services are offered.
  const services = within(dialog).getByRole("list", { name: "Services" });
  expect(services).not.toHaveTextContent("Old API");
  await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Researcher");
  expect(within(dialog).getByRole("textbox", { name: "Name" })).toHaveValue("researcher");
  await user.type(within(dialog).getByRole("textbox", { name: "Role" }), "Finds urgent issues");
  await user.click(within(services).getByRole("checkbox", { name: /GitHub/ }));
  await user.click(within(dialog).getByRole("switch", { name: "Read my account" }));
  expect(create).toBeEnabled();
  await user.click(create);
  await waitFor(() =>
    expect(onCreated).toHaveBeenCalledWith(
      expect.objectContaining({ id: "agent-new", home_conversation_id: home }),
    ),
  );
  expect(posts).toEqual([
    {
      name: "researcher",
      description: "Finds urgent issues",
      services: ["github"],
      account_read: true,
    },
  ]);
});

it("sends an optional display name and persona", async () => {
  const { onCreated, user } = renderDialog();
  const dialog = screen.getByRole("dialog", { name: "New agent" });
  await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "writer");
  await user.type(within(dialog).getByRole("textbox", { name: /Display name/ }), "Luna");
  await user.type(within(dialog).getByRole("textbox", { name: "Role" }), "Drafts notes");
  const persona = within(dialog).getByRole("textbox", { name: /Persona/ });
  expect(persona).toHaveAttribute("placeholder", "e.g. warm, concise, uses emoji sparingly");
  await user.type(persona, "Warm, concise.");
  await user.click(within(dialog).getByRole("button", { name: "Create agent" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(posts).toEqual([
    {
      name: "writer",
      display_name: "Luna",
      description: "Drafts notes",
      persona: "Warm, concise.",
      services: [],
      account_read: false,
    },
  ]);
});

it("rejects an invalid name before calling the server and shows server refusals", async () => {
  const { onCreated, user } = renderDialog();
  const dialog = screen.getByRole("dialog", { name: "New agent" });
  await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "-bad");
  await user.type(within(dialog).getByRole("textbox", { name: "Role" }), "Anything");
  await user.click(within(dialog).getByRole("button", { name: "Create agent" }));
  expect(await within(dialog).findByText(/Use 1 to 32 lowercase letters/)).toBeVisible();
  expect(posts).toEqual([]);

  respond = () =>
    new Response(JSON.stringify({ message: "A live agent already uses that name" }), {
      status: 409,
    });
  const name = within(dialog).getByRole("textbox", { name: "Name" });
  await user.clear(name);
  await user.type(name, "writer");
  await user.click(within(dialog).getByRole("button", { name: "Create agent" }));
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(
    "A live agent already uses that name",
  );
  expect(onCreated).not.toHaveBeenCalled();
});
