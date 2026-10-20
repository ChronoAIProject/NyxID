import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { AllocateDialog } from "./allocate-dialog";
import type { DownstreamService } from "@/types/api";

const api = vi.hoisted(() => ({
  post: vi.fn(),
  lookupFailed: false,
  retry: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({ api: { post: api.post } }));
vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({
    data: {
      users: [
        {
          id: "owner",
          display_name: "Alex",
          email: "alex@example.com",
          is_active: true,
        },
      ],
    },
    isError: api.lookupFailed,
    isFetching: false,
    refetch: api.retry,
  }),
}));
vi.mock("@/hooks/use-rbac", () => ({
  useGroups: () => ({
    data: { groups: [] },
    isError: false,
    isFetching: false,
  }),
}));
const services = [
  { id: "a", slug: "openai", name: "OpenAI", is_active: true },
  { id: "b", slug: "anthropic", name: "Anthropic", is_active: true },
] as DownstreamService[];
const issued = {
  batch_id: "batch",
  created_count: 1,
  activated_count: 0,
  pending_activation_count: 1,
  recipients: [],
};
function setup(
  extra: Partial<React.ComponentProps<typeof AllocateDialog>> = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    services,
    servicesError: false,
    servicesLoading: false,
    retryServices: vi.fn(),
    canRecurring: true,
    ...extra,
  };
  const view = render(
    <QueryClientProvider client={client}>
      <AllocateDialog {...props} />
    </QueryClientProvider>,
  );
  return { ...view, client, invalidate, props };
}
async function recipient() {
  await userEvent.click(screen.getByText("Alex"));
}
async function choose(label: string, option: string) {
  await userEvent.click(screen.getByRole("combobox", { name: label }));
  await userEvent.click(screen.getByRole("option", { name: option }));
}
beforeEach(() => {
  vi.clearAllMocks();
  api.lookupFailed = false;
  api.post.mockResolvedValue(issued);
});
it("requires an explicit recipient and issues exact shared caps through the live hook", async () => {
  const { invalidate } = setup();
  expect(
    screen.getByText("Service limits (optional)").parentElement,
  ).toHaveAttribute("open");
  const submit = screen.getByRole("button", { name: "Allocate credits" });
  expect(submit).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Credits per recipient"), {
    target: { value: "10000.000000000001" },
  });
  expect(submit).toBeDisabled();
  await recipient();
  await userEvent.click(
    screen.getByRole("button", { name: "Add service limit" }),
  );
  fireEvent.change(screen.getByLabelText("Group label 1 (optional)"), {
    target: { value: "LLM" },
  });
  await userEvent.click(screen.getByText("OpenAI"));
  await userEvent.click(screen.getByText("Anthropic"));
  fireEvent.change(screen.getByLabelText("Credit ceiling 1"), {
    target: { value: "300.000000000001" },
  });
  await waitFor(() => expect(submit).toBeEnabled());
  await userEvent.click(submit);
  await waitFor(() => expect(api.post).toHaveBeenCalledOnce());
  expect(api.post).toHaveBeenCalledWith(
    "/admin/credits/allocations",
    expect.objectContaining({
      amount: "10000.000000000001",
      target_user_ids: ["owner"],
      service_limits: [
        { label: "LLM", service_refs: ["a", "b"], amount: "300.000000000001" },
      ],
      request_id: expect.any(String),
    }),
  );
  await waitFor(() =>
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["admin", "credits"] }),
  );
  expect(invalidate).toHaveBeenCalledWith({ queryKey: ["billing", "credits"] });
});
it("round trips modes and rejects invalid recurring expiry without a request", async () => {
  setup();
  await recipient();
  fireEvent.change(screen.getByLabelText("Credits per recipient"), {
    target: { value: "0.3" },
  });
  await choose("Repeat allocation", "Monthly");
  await choose("Expiry for each issuance", "After a number of days");
  for (const value of ["", "0", "1.5", "3651"]) {
    fireEvent.change(screen.getByLabelText("Days until expiry"), {
      target: { value },
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Allocate credits" }),
      ).toBeDisabled(),
    );
    fireEvent.submit(
      screen.getByRole("button", { name: "Allocate credits" }).closest("form")!,
    );
    expect(
      await screen.findByText("Enter 1 to 3,650 days"),
    ).toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();
  }
  await choose("Expiry for each issuance", "Never");
  await choose("Repeat allocation", "One time");
  expect(screen.getByLabelText("Expiry (optional)")).toBeInTheDocument();
  await choose("Repeat allocation", "Weekly");
  await choose("Expiry for each issuance", "End of period");
  api.post.mockResolvedValue({
    id: "s",
    amount_credits: 0,
    amount_micros: 300000,
    amount: "0.3",
    recurrence: "weekly",
    expiry: { kind: "end_of_period" },
    target_kind: "selected_users",
    target_user_ids: ["owner"],
    scope: { all_services: true, service_ids: [], service_slugs: [] },
    is_active: true,
    skipped_periods: 0,
    created_by: "admin",
    created_at: "2026-01-01",
    updated_at: "2026-01-01",
  });
  await userEvent.click(
    screen.getByRole("button", { name: "Allocate credits" }),
  );
  await waitFor(() =>
    expect(api.post).toHaveBeenCalledWith(
      "/admin/credits/allocations/schedules",
      expect.objectContaining({
        amount: "0.3",
        recurrence: "weekly",
        target_user_ids: ["owner"],
        expiry: { kind: "end_of_period" },
      }),
    ),
  );
});
it("keeps all-services issuance usable during catalog failure and fences scoped submission", async () => {
  const { props } = setup({ servicesError: true });
  await recipient();
  fireEvent.change(screen.getByLabelText("Credits per recipient"), {
    target: { value: "20" },
  });
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Allocate credits" }),
    ).toBeEnabled(),
  );
  await userEvent.click(
    screen.getByRole("switch", { name: "All services eligible" }),
  );
  expect(
    screen.getByRole("button", { name: "Allocate credits" }),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(props.retryServices).toHaveBeenCalledOnce();
  fireEvent.submit(
    screen.getByRole("button", { name: "Allocate credits" }).closest("form")!,
  );
  expect(api.post).not.toHaveBeenCalled();
});
it("reuses the request UUID after ambiguous retry and guards pending close and submit", async () => {
  const { props } = setup();
  await recipient();
  fireEvent.change(screen.getByLabelText("Credits per recipient"), {
    target: { value: "20" },
  });
  api.post.mockRejectedValueOnce(new Error("Connection lost"));
  await userEvent.click(
    screen.getByRole("button", { name: "Allocate credits" }),
  );
  await screen.findByText(/Connection lost.*Retry/);
  const key = api.post.mock.calls[0]![1].request_id;
  let finish!: (v: unknown) => void;
  api.post.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Allocate credits" }),
  );
  fireEvent.submit(
    screen.getByRole("button", { name: "Allocate credits" }).closest("form")!,
  );
  await userEvent.keyboard("{Escape}");
  expect(api.post).toHaveBeenCalledTimes(2);
  expect(api.post.mock.calls[1]![1].request_id).toBe(key);
  expect(props.onOpenChange).not.toHaveBeenCalled();
  finish(issued);
  await waitFor(() => expect(props.onOpenChange).toHaveBeenCalledWith(false));
});
