import type { ComponentProps } from "react";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentConnections } from "./agent-connections";
import { useAuthStore } from "@/stores/auth-store";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import type { KeyInfo } from "@/types/keys";
import type { User } from "@/types/api";

const state = vi.hoisted(() => ({
  inventory: [] as import("@/types/keys").KeyInfo[],
  error: false,
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: state.error ? undefined : state.inventory,
    isPending: false,
    isLoading: false,
    isFetching: false,
    isError: state.error,
    dataUpdatedAt: 1,
    refetch: vi.fn(),
  }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, ...props }: ComponentProps<"a"> & { to: string }) => (
    <a href={to} {...props} />
  ),
}));
const catalog = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const ids = [
  "11111111-1111-4111-8111-111111111111",
  "22222222-2222-4222-8222-222222222222",
];
const group = `catalog:${catalog}`;
const agent = {
  id: "nyxbot",
  kind: "nyxbot",
  status: "idle",
  services: [],
} as unknown as AssistantAgent;
const key = (id: string, label: string): KeyInfo =>
  ({
    id,
    label,
    slug: label.toLowerCase(),
    catalog_service_id: catalog,
    catalog_service_slug: "google-drive",
    catalog_service_name: "Google Drive",
    is_active: true,
    credential_source: { type: "personal" },
  }) as KeyInfo;
let writes: { ordered: string[]; expected_version: number }[];
let preference: {
  groups: { group: string; ordered: string[] }[];
  version: number;
  updated_at: null;
};
let conflict: boolean;
let client: QueryClient;
beforeEach(() => {
  state.inventory = [key(ids[0]!, "Work"), key(ids[1]!, "Personal")];
  state.error = false;
  writes = [];
  preference = { groups: [], version: 3, updated_at: null };
  conflict = false;
  useAuthStore.setState({ user: { id: "owner" } as User });
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.method === "PUT") {
        const body = JSON.parse(String(init.body)) as (typeof writes)[number];
        writes.push(body);
        if (conflict)
          return new Response(
            JSON.stringify({
              error: "conflict",
              error_code: 1,
              message: "Order changed",
            }),
            { status: 409 },
          );
        preference = {
          groups: [{ group, ordered: body.ordered }],
          version: preference.version + 1,
          updated_at: null,
        };
      }
      return new Response(JSON.stringify(preference), {
        headers: { "content-type": "application/json" },
      });
    }),
  );
});
afterEach(() => {
  client.clear();
  vi.unstubAllGlobals();
  useAuthStore.setState({ user: null });
});
function mount(subject = agent) {
  return render(
    <QueryClientProvider client={client}>
      <AgentConnections agent={subject} />
    </QueryClientProvider>,
  );
}
async function edit() {
  fireEvent.click(screen.getByText("Google Drive", { exact: true }));
  const edit = screen.getByRole("button", { name: "Edit connection order" });
  await waitFor(() => expect(edit).toBeEnabled());
  fireEvent.click(edit);
  fireEvent.click(screen.getByRole("button", { name: "Move Personal up" }));
}
it("saves account-wide order from Details with the live version and survives reopening", async () => {
  const view = mount();
  await edit();
  fireEvent.click(screen.getByRole("button", { name: "Save order" }));
  await waitFor(() =>
    expect(writes).toEqual([
      { ordered: [ids[1], ids[0]], expected_version: 3 },
    ]),
  );
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Save order" }),
    ).not.toBeInTheDocument(),
  );
  view.unmount();
  mount();
  fireEvent.click(screen.getByText("Google Drive", { exact: true }));
  const rows = within(
    screen.getByRole("list", { name: "Google Drive connections" }),
  ).getAllByRole("listitem");
  expect(within(rows[0]!).getByText("Personal", { exact: true })).toBeVisible();
  expect(screen.getByText(/applies across your agents/)).toBeVisible();
});
it("requires an explicit reload on a version conflict", async () => {
  conflict = true;
  mount();
  await edit();
  fireEvent.click(screen.getByRole("button", { name: "Save order" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "changed elsewhere",
  );
  expect(writes).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Save order" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Reload order" })).toBeEnabled();
});
it("keeps an unsaved order visible when inventory refresh fails", async () => {
  const view = mount();
  await edit();
  state.error = true;
  view.rerender(
    <QueryClientProvider client={client}>
      <AgentConnections agent={agent} />
    </QueryClientProvider>,
  );
  expect(screen.getByRole("button", { name: "Save order" })).toBeDisabled();
  expect(
    screen.getByRole("list", { name: "Google Drive connections" }),
  ).toHaveTextContent("Personal");
  expect(writes).toHaveLength(0);
});
it("shows only explicitly granted specialist connections and offers no account-wide editor", () => {
  mount({ ...agent, kind: "specialist", services: [ids[1]!] });
  fireEvent.click(screen.getByText("Google Drive", { exact: true }));
  const list = screen.getByRole("list", { name: "Google Drive connections" });
  expect(list).toHaveTextContent("Personal");
  expect(list).not.toHaveTextContent("Work");
  expect(
    screen.queryByRole("button", { name: "Edit connection order" }),
  ).not.toBeInTheDocument();
  expect(window.fetch).not.toHaveBeenCalled();
});
it("makes a destroyed agent's order read-only", async () => {
  mount({ ...agent, status: "destroyed" });
  fireEvent.click(screen.getByText("Google Drive", { exact: true }));
  expect(
    screen.getByRole("button", { name: "Edit connection order" }),
  ).toBeDisabled();
  expect(writes).toHaveLength(0);
});
