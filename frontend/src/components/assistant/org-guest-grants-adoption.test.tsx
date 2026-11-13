import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { OrgGuestGrantsAdoption } from "./org-guest-grants-adoption";
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: { user: { id: string } }) => unknown) =>
    select({ user: { id: "admin" } }),
}));
let client: QueryClient;
let writes: unknown[];
let conflict: boolean;
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  writes = [];
  conflict = false;
  globalThis.__nyxidAssistantHttpMock = ({ init }) => {
    if (init.method === "POST") {
      writes.push(JSON.parse(String(init.body)));
      if (conflict)
        return new Response(
          JSON.stringify({ message: "Guest settings changed" }),
          { status: 409 },
        );
    }
    return new Response(
      JSON.stringify({
        ready: true,
        pending: writes.length === 0 || conflict,
        review: "bound-review",
        bots: [
          { id: "one", name: "Office helper", status: "active" },
          { id: "two", name: "Support helper", status: "disconnected" },
        ],
        services: [
          {
            id: "secret-id",
            label: "Calendar",
            access: "read",
            limited_operations: true,
          },
        ],
      }),
    );
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
});
function mount(disabled = false) {
  render(
    <QueryClientProvider client={client}>
      <OrgGuestGrantsAdoption agentId="agent" disabled={disabled} />
    </QueryClientProvider>,
  );
}
it("reviews named bots and resulting saved access, then removes the notice", async () => {
  mount();
  await userEvent.click(
    await screen.findByRole("button", {
      name: "Use these guest settings for all linked organization bots",
    }),
  );
  const dialog = await screen.findByRole("dialog");
  expect(dialog).toHaveTextContent("Office helper, Support helper");
  expect(dialog).toHaveTextContent("Calendar: Look things up only");
  expect(dialog).toHaveTextContent("Selected operations only");
  expect(dialog).not.toHaveTextContent("secret-id");
  expect(writes).toEqual([]);
  expect(within(dialog).queryByRole("checkbox")).not.toBeInTheDocument();
  await userEvent.click(
    within(dialog).getByRole("button", {
      name: "Apply reviewed guest settings",
    }),
  );
  await waitFor(() => expect(writes).toEqual([{ review: "bound-review" }]));
  await waitFor(() =>
    expect(
      screen.queryByText(
        "Existing guest limits are kept. Review them in Grants.",
      ),
    ).not.toBeInTheDocument(),
  );
});
it("requires saving dirty Grants before adoption", async () => {
  mount(true);
  expect(
    await screen.findByRole("button", {
      name: "Use these guest settings for all linked organization bots",
    }),
  ).toBeDisabled();
  expect(writes).toEqual([]);
});
it("requires a fresh review after a conflict", async () => {
  conflict = true;
  mount();
  await userEvent.click(
    await screen.findByRole("button", {
      name: "Use these guest settings for all linked organization bots",
    }),
  );
  await userEvent.click(
    within(await screen.findByRole("dialog")).getByRole("button", {
      name: "Apply reviewed guest settings",
    }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Reload and review",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(writes).toHaveLength(1);
});
