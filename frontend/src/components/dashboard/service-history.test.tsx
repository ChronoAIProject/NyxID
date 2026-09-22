import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import {
  ServiceAuthorshipFooter,
  ServiceHistory,
  ArchivedServiceHistory,
} from "./service-history";
import { TooltipProvider } from "@/components/ui/tooltip";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const fetchMock = vi.fn<typeof fetch>();
const actor = {
  kind: "person",
  id: "editor",
  name: "Ada",
  person_id: "editor",
  api_key_id: null,
  app_id: null,
};
const timestamp = "2026-09-16T12:30:00Z";
const summary = {
  actor,
  at: timestamp,
  action: "service.updated",
  change_group_id: "group",
};
const event = {
  id: "event",
  schema_version: 1,
  action: "service.updated",
  action_label: "Service updated",
  actor,
  committed_at: timestamp,
  additional_changes: true,
  audit_status: "pending",
  changes: [
    { field: "admin_only", label: "Admin only", before: false, after: true },
    { field: "credential", label: "Credential", before: null, after: null },
    {
      field: "token_scopes",
      label: "OAuth scopes",
      before: [],
      after: ["read:user"],
    },
    {
      field: "default_request_headers",
      label: "Default request headers",
      before: null,
      after: { count: 1, names: ["accept"] },
    },
    {
      field: "ws_frame_injections",
      label: "WebSocket authentication rules",
      before: null,
      after: { count: 1, directions: ["downstream"] },
    },
  ],
};
const page = {
  service_id: "service",
  next_cursor: null,
  tracked_since: timestamp,
  legacy: false,
  deleted: false,
  groups: [{ id: "group", events: [event] }],
};
function wrapper(
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
) {
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>
      <TooltipProvider>{children}</TooltipProvider>
    </QueryClientProvider>
  );
}
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    params,
    children,
  }: {
    params: { keyId: string };
    children: React.ReactNode;
  }) => <a href={`/keys/${params.keyId}`}>{children}</a>,
}));
beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
  useAuthStore.setState({ user: { id: "owner" } as User });
  fetchMock.mockImplementation(async (url) =>
    String(url).includes("/options/")
      ? new Response("{}", { status: 503 })
      : new Response(JSON.stringify(page)),
  );
});
afterEach(() => vi.unstubAllGlobals());

describe("service authorship and history", () => {
  it("hides unauthorized metadata and gives honest legacy and new-service footers", () => {
    const view = render(<ServiceAuthorshipFooter />, { wrapper: wrapper() });
    expect(
      screen.queryByLabelText("Service authorship"),
    ).not.toBeInTheDocument();
    view.rerender(
      <ServiceAuthorshipFooter
        authorship={{ created_by: null, last_change: null }}
      />,
    );
    expect(screen.getByText("Creator not recorded")).toBeInTheDocument();
    expect(screen.getByText("Earlier edits not recorded")).toBeInTheDocument();
    view.rerender(
      <ServiceAuthorshipFooter
        authorship={{ created_by: summary, last_change: null }}
      />,
    );
    expect(screen.getByText("No edits since creation")).toBeInTheDocument();
    expect(screen.getByLabelText("Service authorship")).toHaveClass(
      "items-end",
      "text-right",
      "min-w-0",
    );
    expect(screen.getByText(/Created by Ada/)).toHaveClass("break-words");
    expect(screen.getByRole("time")).toHaveAttribute("tabindex", "0");
  });

  it("renders embedded labels and approved metadata even when options are unavailable", async () => {
    render(<ServiceHistory serviceId="service" />, { wrapper: wrapper() });
    expect(screen.getByRole("status")).toHaveTextContent(
      "Loading service history",
    );
    await screen.findByText(
      "Other service settings updated. Additional change details are unavailable.",
    );
    await userEvent.click(
      screen.getByText("Service updated", { selector: "summary span" }),
    );
    expect(screen.getByText("No → Yes")).toBeVisible();
    expect(screen.getByText("Changed; values omitted")).toBeVisible();
    expect(screen.getByText("No reviewed scopes → read:user")).toBeVisible();
    expect(screen.getByText("Not recorded → 1 header (accept)")).toBeVisible();
    expect(
      screen.getByText("Not recorded → 1 rule (downstream)"),
    ).toBeVisible();
    const exact = new Date(timestamp).toLocaleString(undefined, {
      dateStyle: "full",
      timeStyle: "long",
    });
    expect(screen.getByText(exact, { selector: "time" })).toBeVisible();
  });

  it("hides cached rows immediately after an authorization failure", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(<ServiceHistory serviceId="service" />, {
      wrapper: wrapper(client),
    });
    await screen.findByText(/Other service settings updated/);
    fetchMock.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({ message: "Not found", error_code: 1000 }),
          { status: 404 },
        ),
    );
    await act(() =>
      client.invalidateQueries({ queryKey: ["service-history"] }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "currently permitted organization admins",
    );
    expect(
      screen.queryByText(/Other service settings updated/),
    ).not.toBeInTheDocument();
  });

  it("isolates signed-in identities and supports retained, legacy, empty and unknown actions", async () => {
    const view = render(<ServiceHistory serviceId="service" />, {
      wrapper: wrapper(),
    });
    await screen.findByText(/Other service settings updated/);
    fetchMock.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({ ...page, groups: [], deleted: true, legacy: true }),
        ),
    );
    act(() =>
      useAuthStore.setState({ user: { id: "different-owner" } as User }),
    );
    expect(
      screen.queryByText(/Other service settings updated/),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByText(/Its recorded history is retained/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Creator and earlier edits may not have been recorded/),
    ).toBeInTheDocument();
    expect(screen.getByText("No recorded changes yet.")).toBeInTheDocument();
    fetchMock.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({
            ...page,
            service_id: "retired",
            groups: [
              {
                id: "old",
                events: [
                  {
                    ...event,
                    action: "service.retired_action",
                    action_label: undefined,
                  },
                ],
              },
            ],
          }),
        ),
    );
    view.rerender(<ServiceHistory serviceId="retired" />);
    await waitFor(() =>
      expect(
        screen.getByText("Service updated", { selector: "summary span" }),
      ).toBeInTheDocument(),
    );
  });
});

it.each([
  ["api_key", "API key"],
  ["service_account", "Service account"],
  ["app", "Application"],
  ["system", "System"],
])("labels the %s identity rather than implying a person", (kind, label) => {
  render(
    <ServiceAuthorshipFooter
      authorship={{
        created_by: { ...summary, actor: { ...actor, kind, person_id: null } },
        last_change: null,
      }}
    />,
    { wrapper: wrapper() },
  );
  expect(
    screen.getByText(`Created by Ada (${label})`, { exact: false }),
  ).toBeInTheDocument();
});

it("paginates whole operations and summarizes Delete after earlier local changes", async () => {
  fetchMock.mockImplementation(async (url) => {
    if (String(url).includes("/options/"))
      return new Response("{}", { status: 503 });
    return new Response(
      JSON.stringify(
        String(url).includes("cursor=older")
          ? {
              ...page,
              groups: [
                {
                  id: "creation",
                  events: [
                    {
                      ...event,
                      id: "created",
                      action: "service.created",
                      action_label: "Service created",
                    },
                    { ...event, id: "initialization" },
                  ],
                },
              ],
            }
          : {
              ...page,
              next_cursor: "older",
              groups: [
                {
                  id: "delete",
                  events: [
                    {
                      ...event,
                      id: "disable",
                      action: "service.disabled",
                      action_label: "Service disabled",
                    },
                    {
                      ...event,
                      id: "delete",
                      action: "service.deleted",
                      action_label: "Service deleted",
                      changes: [
                        {
                          field: "default_request_headers",
                          label: "Headers",
                          before: { count: 1, names: ["accept"] },
                          after: { count: 1, names: ["accept"] },
                        },
                      ],
                    },
                  ],
                },
              ],
            },
      ),
    );
  });
  render(<ServiceHistory serviceId="service" />, { wrapper: wrapper() });
  await userEvent.click(
    await screen.findByText("Service deleted", { selector: "summary span" }),
  );
  expect(screen.getAllByText("Changed; values omitted").length).toBeGreaterThan(
    0,
  );
  expect(
    screen.queryByText("1 header (accept) → 1 header (accept)"),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Load older changes" }),
  );
  expect(
    await screen.findByText("Service created", { selector: "summary span" }),
  ).toBeInTheDocument();
  expect(document.querySelectorAll("details")).toHaveLength(2);
  expect(
    screen.queryByRole("button", { name: "Load older changes" }),
  ).not.toBeInTheDocument();
  expect(
    fetchMock.mock.calls.some(([url]) => String(url).includes("cursor=older")),
  ).toBe(true);
});

it("offers a persistent UUID entry for deleted service history", async () => {
  fetchMock.mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          services: [
            {
              service_id: "retained-uuid",
              service_slug: "renamed-service",
              last_changed_at: timestamp,
            },
          ],
          next_cursor: null,
        }),
      ),
  );
  render(<ArchivedServiceHistory />, { wrapper: wrapper() });
  expect(fetchMock).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Deleted service history" }),
  );
  expect(
    await screen.findByRole("link", { name: /renamed-service/ }),
  ).toHaveAttribute("href", "/keys/retained-uuid");
  expect(String(fetchMock.mock.calls[0]![0])).toContain(
    "/keys/history/archived",
  );
});

it("refreshes archive after a mutation when no timeline is mounted", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  fetchMock.mockImplementation(
    async () =>
      new Response(JSON.stringify({ services: [], next_cursor: null })),
  );
  render(<ArchivedServiceHistory />, { wrapper: wrapper(client) });
  await userEvent.click(
    screen.getByRole("button", { name: "Deleted service history" }),
  );
  await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
  fetchMock.mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          services: [
            {
              service_id: "newly-deleted",
              service_slug: "Just deleted",
              last_changed_at: timestamp,
            },
          ],
          next_cursor: null,
        }),
      ),
  );
  await act(() =>
    client
      .getMutationCache()
      .build(client, { mutationFn: async () => undefined })
      .execute(undefined),
  );
  expect(
    await screen.findByRole("link", { name: /Just deleted/ }),
  ).toHaveAttribute("href", "/keys/newly-deleted");
});
