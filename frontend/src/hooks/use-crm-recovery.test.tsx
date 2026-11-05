import { useState, type PropsWithChildren } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { useCrmRecord, useCrmActivityHistory } from "@/hooks/use-crm";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: request }));
const record = {
  id: "record-a",
  owner_id: "org-a",
  object_key: "company",
  revision: 1,
  values: { name: "Original" },
  links: {},
  overrides: [],
  archived: false,
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  return {
    client,
    wrapper: ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  };
}
function Editor({ name }: { name: string }) {
  const [draft, setDraft] = useState(name);
  return (
    <input
      aria-label="Draft"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
    />
  );
}
function Surface({ owner = "org-a" }: { owner?: string }) {
  const query = useCrmRecord("record-a", owner);
  return (
    <div>
      <button onClick={() => void query.refetch()}>Refresh</button>
      {query.isError ? <p>Refresh failed</p> : null}
      {query.data ? (
        <Editor key={query.data.id} name={String(query.data.values.name)} />
      ) : (
        <p>No record</p>
      )}
    </div>
  );
}
beforeEach(() => {
  request.mockReset();
  useAuthStore.setState({ user: { id: "person-a" } as User });
});
it("retains a dirty draft through a transient refetch and removes it after an actual refusal", async () => {
  const { wrapper, client } = setup();
  request.mockResolvedValue(record);
  render(<Surface />, { wrapper });
  fireEvent.change(await screen.findByLabelText("Draft"), {
    target: { value: "Unsaved" },
  });
  request.mockRejectedValue(
    new ApiError(503, {
      error: "unavailable",
      error_code: 1000,
      message: "Try again",
    }),
  );
  fireEvent.click(screen.getByText("Refresh"));
  await screen.findByText("Refresh failed");
  expect(screen.getByLabelText("Draft")).toHaveValue("Unsaved");
  client.setQueryData(
    ["crm", "org-a", "record", "unrelated", "actor", "person-a"],
    { ...record, id: "unrelated" },
  );
  client.setQueryData(["crm", "org-a", "table", {}, "actor", "person-a"], {
    rows: [record],
    total_count: 1,
  });
  request.mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 1000,
      message: "Gone",
    }),
  );
  fireEvent.click(screen.getByText("Refresh"));
  await waitFor(() =>
    expect(screen.queryByLabelText("Draft")).not.toBeInTheDocument(),
  );
  expect(
    client.getQueryData([
      "crm",
      "org-a",
      "record",
      "unrelated",
      "actor",
      "person-a",
    ]),
  ).toBeDefined();
  expect(
    client.getQueryData(["crm", "org-a", "table", {}, "actor", "person-a"]),
  ).toBeUndefined();
});
it("never reuses the previous actor's same-owner cache or another owner's draft", async () => {
  const { wrapper } = setup();
  request.mockResolvedValue(record);
  const view = render(<Surface />, { wrapper });
  fireEvent.change(await screen.findByLabelText("Draft"), {
    target: { value: "Private draft" },
  });
  request.mockImplementation(() => new Promise(() => undefined));
  act(() => useAuthStore.setState({ user: { id: "person-b" } as User }));
  await waitFor(() =>
    expect(screen.queryByLabelText("Draft")).not.toBeInTheDocument(),
  );
  view.rerender(<Surface owner="org-b" />);
  expect(screen.queryByDisplayValue("Private draft")).not.toBeInTheDocument();
});
it("pages activity, recovers a failed older page, and refreshes the first page", async () => {
  const { wrapper } = setup();
  const activity = (id: string) => ({
    id,
    record_id: "record-a",
    object_key: "company",
    kind: "note",
    actor: { kind: "human", user_id: "person-a" },
    summary: {},
    body: id,
    created_at: "2026-11-04T00:00:00Z",
  });
  request.mockResolvedValueOnce({
    activities: [activity("new")],
    next_cursor: "older",
  });
  const hook = renderHook(() => useCrmActivityHistory("record-a", "org-a"), {
    wrapper,
  });
  await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
  request.mockRejectedValueOnce(new Error("Network"));
  await act(async () => {
    await hook.result.current.fetchNextPage();
  });
  await waitFor(() =>
    expect(hook.result.current.isFetchNextPageError).toBe(true),
  );
  request.mockResolvedValueOnce({
    activities: [activity("old")],
    next_cursor: null,
  });
  await act(async () => {
    await hook.result.current.fetchNextPage();
  });
  await waitFor(() =>
    expect(
      hook.result.current.data?.pages.flatMap((page) =>
        page.activities.map((row) => row.id),
      ),
    ).toEqual(["new", "old"]),
  );
  expect(request).toHaveBeenLastCalledWith(
    "/assistant/crm/records/record-a/activities?cursor=older&owner_id=org-a",
  );
  request
    .mockResolvedValueOnce({
      activities: [activity("newer"), activity("new")],
      next_cursor: "older",
    })
    .mockResolvedValueOnce({
      activities: [activity("old")],
      next_cursor: null,
    });
  await act(async () => {
    await hook.result.current.refetch();
  });
  await waitFor(() =>
    expect(hook.result.current.data?.pages[0]?.activities[0]?.id).toBe("newer"),
  );
});
