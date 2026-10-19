import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PluginsView } from "./plugins-view";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

// The Skills tab through the real skill hooks, schemas and request path
// (assistantJson -> fetch -> ApiError). Only the agent roster and the
// connectors tab's data hooks are stubbed.
const roster = vi.hoisted(() => ({
  agents: [
    { id: "nyxbot", kind: "nyxbot", name: "NyxBot", display_name: null, status: "idle", can_maintain: true },
    { id: "writer", kind: "specialist", name: "writer", display_name: "Writer", status: "idle", can_maintain: true },
    { id: "gone", kind: "specialist", name: "gone", display_name: null, status: "destroyed", can_maintain: true },
  ],
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: roster.agents }, isPending: false, error: null }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalog: () => ({ data: [], isLoading: false, error: null, refetch: vi.fn() }),
  useKeys: () => ({ data: [], isLoading: false, error: null, refetch: vi.fn() }),
}));

const root = "/api/v1/assistant/nyxagent";
const pin = (id: string, name: string, version: string, sha: string) =>
  ({ source: "ornn", skill_id: id, name, version, sha256: sha.repeat(64), dependencies: [] });
type Reply = { status: number; body: unknown };
let pins: Record<string, Array<ReturnType<typeof pin>>>;
let putFailure: Reply | undefined;
let fetchMock: ReturnType<typeof vi.fn>;

const json = ({ status, body }: Reply) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: { id: "owner", capabilities: { enabled_features: [] } } as unknown as User });
  pins = { nyxbot: [pin("research", "research", "1.0.0", "a")], writer: [] };
  putFailure = undefined;
  fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? "GET";
    const skills = /^\/api\/v1\/assistant\/nyxagent\/agents\/([^/]+)\/skills$/.exec(url);
    if (skills && method === "GET") {
      const id = skills[1]!;
      return json({ status: 200, body: { agent_id: id, revision: 7, skills: pins[id] ?? [], metadata: {} } });
    }
    if (skills && method === "PUT") {
      if (putFailure) return json(putFailure);
      const sent = JSON.parse(String(init!.body)) as { skills: Array<ReturnType<typeof pin>> };
      pins[skills[1]!] = sent.skills;
      return json({ status: 200, body: { agent_id: skills[1], revision: 8, skills: sent.skills, metadata: {} } });
    }
    if (url === `${root}/skills/catalog?q=writing&page=1`) {
      return json({ status: 200, body: { page: 1, total_pages: 1, items: [
        { id: "research", name: "research", description: "Research guide" },
        { id: "style", name: "style", description: "House style" },
      ] } });
    }
    if (url === `${root}/skills/catalog?skill=style&page=1`) {
      return json({ status: 200, body: { page: 1, total_pages: 1, items: [
        { version: "3.0.0", sha256: "c".repeat(64), deprecated: true },
        { version: "2.1.0", sha256: "b".repeat(64), deprecated: false },
      ] } });
    }
    if (url === `${root}/skills/catalog?skill=style&version=2.1.0`) {
      return json({ status: 200, body: { reference: pin("style", "style", "2.1.0", "b"), description: "House style", size_bytes: 900 } });
    }
    return json({ status: 404, body: { error: "not_found", error_code: 1004, message: "Not found" } });
  });
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useAuthStore.setState({ user: null });
});

async function openSkills() {
  const user = userEvent.setup();
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } })}>
    <PluginsView />
  </QueryClientProvider>);
  await user.click(screen.getByRole("tab", { name: "Skills" }));
  return user;
}

function puts(agent: string) {
  return fetchMock.mock.calls
    .filter(([url, init]) => url === `${root}/agents/${agent}/skills` && (init as RequestInit | undefined)?.method === "PUT")
    .map(([, init]) => JSON.parse(String((init as RequestInit).body)) as unknown);
}

it("manages NyxBot by default and lists its attached skills", async () => {
  await openSkills();
  expect(screen.getByRole("combobox", { name: "Agent for skills" })).toHaveTextContent("NyxBot");
  const list = await screen.findByRole("region", { name: "Attached skills" });
  expect(within(list).getByText("Attached to NyxBot · 1/16")).toBeVisible();
  expect(within(list).getByText("research")).toBeVisible();
  expect(within(list).getByText("1.0.0")).toBeVisible();
});

it("keeps the picker mounted while switching agents", async () => {
  const user = await openSkills();
  const picker = screen.getByRole("combobox", { name: "Agent for skills" });
  await user.click(picker);
  // Destroyed agents keep their pins but are not offered for changes.
  expect(screen.queryByRole("option", { name: "gone" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("option", { name: "Writer" }));
  expect(screen.getByRole("combobox", { name: "Agent for skills" })).toBe(picker);
  expect(picker).toHaveTextContent("Writer");
  expect(await screen.findByText("Attached to Writer · 0/16")).toBeVisible();
});

it("marks attached catalog skills and attaches others at an exact version and sha256", async () => {
  const user = await openSkills();
  await screen.findByText("Attached to NyxBot · 1/16");
  await user.type(screen.getByPlaceholderText("Search the catalog..."), "writing");
  // research is already pinned, so its card is inert.
  expect(await screen.findByText("Research guide")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Attach research" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Attach style" }));
  await waitFor(() => expect(puts("nyxbot")).toHaveLength(1));
  // The newest non-deprecated version, pinned from its verified preview.
  expect(puts("nyxbot")[0]).toEqual({
    expected_revision: 7,
    // Pins parse with an explicit enabled state; the server accepts it with the flag off.
    skills: [
      { ...pin("research", "research", "1.0.0", "a"), disabled: false },
      { ...pin("style", "style", "2.1.0", "b"), disabled: false },
    ],
  });
  expect(await screen.findByText("Attached to NyxBot · 2/16")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Attach style" })).not.toBeInTheDocument();
});

it("detaches a pin with the observed revision", async () => {
  const user = await openSkills();
  await user.click(await screen.findByRole("button", { name: "Detach research" }));
  await waitFor(() => expect(puts("nyxbot")).toEqual([{ expected_revision: 7, skills: [] }]));
  expect(await screen.findByText("No skills attached. Search the catalog to attach one.")).toBeVisible();
});

it("shows a refused change", async () => {
  putFailure = { status: 409, body: { error: "conflict", error_code: 1009, message: "Agent skills changed; reload and try again" } };
  const user = await openSkills();
  await user.click(await screen.findByRole("button", { name: "Detach research" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Agent skills changed; reload and try again");
  expect(screen.getByText("research")).toBeVisible();
});
