import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { ManagedMetaConnect } from "./managed-meta";

vi.mock("@/stores/auth-store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/stores/auth-store")>()),
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: "owner" } }),
}));

afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.restoreAllMocks();
});

it("keeps each inbox's saved consent session and resets asset confirmation on navigation", async () => {
  const client = new QueryClient();
  const first = "587da031-d973-493f-985a-9765d3c8bde0";
  const second = "2d2c5f73-d1e1-43a8-869b-b8493b472967";
  const key = (id: string) => `meta-inbox:owner:messenger:personal:${id}:direct`;
  sessionStorage.setItem(key("a"), first);
  sessionStorage.setItem(key("b"), second);
  vi.spyOn(api, "get").mockImplementation(async (path) => {
    const id = path.split("/").at(-1);
    return {
      platform: "messenger",
      status: "assets",
      session_id: id,
      bot_id: null,
      assets: [{ id: "123", name: id === first ? "First Page" : "Second Page" }],
    };
  });
  const form = (botId: string) => (
    <QueryClientProvider client={client}>
      <ManagedMetaConnect platform="messenger" label="Support" orgId={null} botId={botId} onConnected={vi.fn()} />
    </QueryClientProvider>
  );
  const { rerender, unmount } = render(form("a"));
  await userEvent.click(await screen.findByRole("radio", { name: /First Page/ }));
  await userEvent.click(screen.getByRole("checkbox"));
  rerender(form("b"));
  expect(await screen.findByRole("radio", { name: /Second Page/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Connect selected inbox" })).toBeDisabled();
  expect(sessionStorage.getItem(key("b"))).toBe(second);
  unmount();
  client.clear();
});
