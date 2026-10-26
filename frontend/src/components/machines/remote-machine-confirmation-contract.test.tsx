import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  RemoteMachineConfirmations,
  ConfirmationDecision,
} from "./remote-machine-confirmations";
import { useAuthStore } from "@/stores/auth-store";
import { remoteMachineConfirmationSchema } from "@/schemas/machines";

const http = vi.hoisted(() => ({ get: vi.fn(), read: vi.fn() }));
vi.mock("@/lib/api-client", async (load) => ({
  ...(await load<typeof import("@/lib/api-client")>()),
  api: { get: http.get },
  apiClient: http.read,
}));
const owner = "owner-one";
const node = "machine-one";
function payload() {
  // Actual owner_confirmations projection, including the redacted review flags.
  return {
    id: "confirmation-one",
    node_id: node,
    principal: {
      kind: "api_key",
      user_id: owner,
      api_key_id: "key-one",
      api_key_credential_id: "child-one",
    },
    grant_id: "grant-one",
    grant_revision: 7,
    assignment_revision: 3,
    runtime_id: "runtime-one",
    runtime_generation: 2,
    operation: "exec",
    parameters_digest: "parameter-digest",
    safe_parameters: [{ name: "Command bytes", value: "42" }],
    review_parameters_available: true,
    review_content_available: false,
    state: "pending",
    created_at: "2026-10-14T08:00:00Z",
    expires_at: "2099-10-14T08:15:00Z",
    decided_at: null,
    owner_only: true,
  };
}
let client: QueryClient;
function mount() {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <RemoteMachineConfirmations machineId={node} />
    </QueryClientProvider>,
  );
}
async function openReview() {
  await userEvent.click(
    await screen.findByRole("button", { name: "Review decision" }),
  );
  expect(screen.getByRole("dialog")).toBeInTheDocument();
}
beforeEach(() => {
  useAuthStore.setState({ user: { id: owner } as never });
});
afterEach(() => {
  cleanup();
  client?.clear();
  vi.resetAllMocks();
});

it("shows the actual safe summary and keeps absent pagination explicitly unknown", async () => {
  const row = payload();
  http.get.mockResolvedValue({ confirmations: [row] });
  http.read.mockResolvedValue({
    confirmation: {
      ...row,
      subject: row.principal,
      status: "approved",
      approved_at: row.created_at,
      approved_until: row.expires_at,
    },
  });
  mount();
  await openReview();
  expect(screen.getAllByText("Command bytes")).toHaveLength(2);
  expect(
    screen.getByText(/Older-page availability is unknown/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", {
      name: /Next confirmations|Previous confirmations/,
    }),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I reviewed this identity/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve one retry" }),
  );
  await waitFor(() => expect(http.read).toHaveBeenCalledOnce());
  expect(http.read.mock.calls[0]?.[0]).toBe(
    `/remote-machines/${node}/confirmations/${row.id}/decision`,
  );
  expect(http.read.mock.calls[0]?.[1].body).toEqual({
    approve: true,
    expected_grant_revision: 7,
    expected_runtime_generation: 2,
    parameters_digest: "parameter-digest",
  });
});

it.each([
  { review_parameters_available: false },
  { review_parameters_available: undefined },
  { safe_parameters: [] },
  { runtime_generation: null },
])("blocks review for incomplete server authority %j", async (change) => {
  http.get.mockResolvedValue({ confirmations: [{ ...payload(), ...change }] });
  mount();
  expect(
    await screen.findByRole("button", { name: "Review decision" }),
  ).toBeDisabled();
  expect(http.read).not.toHaveBeenCalled();
  if ("runtime_generation" in change)
    expect(screen.getByText("Unknown")).toBeInTheDocument();
});

it.each([
  [
    "child",
    {
      principal: { ...payload().principal, api_key_credential_id: "child-two" },
    },
  ],
  ["grant", { grant_id: "grant-two" }],
  ["assignment", { assignment_revision: 4 }],
  ["runtime", { runtime_id: "runtime-two" }],
  ["summary", { safe_parameters: [{ name: "Command bytes", value: "80" }] }],
])(
  "closes an open review when the current %s snapshot changes",
  async (_name, change) => {
    const row = payload();
    http.get.mockResolvedValue({ confirmations: [row] });
    mount();
    await openReview();
    http.get.mockResolvedValue({ confirmations: [{ ...row, ...change }] });
    await act(async () => {
      await client.refetchQueries();
    });
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(http.read).not.toHaveBeenCalled();
  },
);

it("closes a review after a failed live refresh and does not restore its old acknowledgement", async () => {
  http.get.mockResolvedValue({ confirmations: [payload()] });
  mount();
  await openReview();
  http.get.mockRejectedValue(new Error("live access unavailable"));
  await act(async () => {
    await client.refetchQueries();
  });
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  http.get.mockResolvedValue({ confirmations: [payload()] });
  await act(async () => {
    await client.refetchQueries();
  });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(http.read).not.toHaveBeenCalled();
});

it("blocks both direct decision controls with a null runtime generation", () => {
  const row = payload();
  const confirmation = remoteMachineConfirmationSchema.parse({
    ...row,
    subject: row.principal,
    status: row.state,
    runtime_generation: null,
    approved_at: null,
    approved_until: null,
  });
  client = new QueryClient();
  render(
    <QueryClientProvider client={client}>
      <ConfirmationDecision
        machineId={node}
        confirmation={confirmation}
        onClose={() => {}}
      />
    </QueryClientProvider>,
  );
  expect(screen.getByRole("button", { name: "Decline" })).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Approve one retry" }),
  ).toBeDisabled();
});

it("follows global confirmation cursors and clears an old review before changing pages", async () => {
  const cursor =
    "2026-10-14T08:00:00.000Z|f439233e-6dfe-44e4-8241-73d1da023111";
  http.get.mockImplementation(async (url: string) => ({
    confirmations: [
      {
        ...payload(),
        id: url.includes("cursor=")
          ? "older-confirmation"
          : "newer-confirmation",
      },
    ],
    next_cursor: url.includes("cursor=") ? null : cursor,
  }));
  mount();
  await screen.findByText("newer-confirmation");
  await userEvent.click(
    screen.getByRole("button", { name: "Older confirmations" }),
  );
  expect(await screen.findByText("older-confirmation")).toBeInTheDocument();
  expect(
    http.get.mock.calls.some(([url]) =>
      String(url).includes(new URLSearchParams({ cursor }).toString()),
    ),
  ).toBe(true);
  expect(
    screen.getByRole("button", { name: "Older confirmations" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Newer confirmations" }),
  );
  expect(await screen.findByText("newer-confirmation")).toBeInTheDocument();
});

it("displays the exact reviewed authority and expiry inside the decision dialog", async () => {
  http.get.mockResolvedValue({ confirmations: [payload()] });
  mount();
  await openReview();
  const dialog = within(screen.getByRole("dialog"));
  expect(
    dialog.getByText("Grant / assignment revision").nextElementSibling,
  ).toHaveTextContent("7 / 3");
  expect(
    dialog.getByText("Runtime generation").nextElementSibling,
  ).toHaveTextContent("2");
  expect(
    dialog.getByText("Parameter digest").nextElementSibling,
  ).toHaveTextContent("parameter-digest");
  expect(
    dialog.getByText("Waiting approval expires").nextElementSibling,
  ).toHaveTextContent(new Date(payload().expires_at).toLocaleString());
  expect(
    dialog.getByText(/Full request content is unavailable/),
  ).toBeInTheDocument();
});

it("reports approval as waiting for the original caller even before the next read completes", async () => {
  const row = payload();
  http.get.mockResolvedValue({ confirmations: [row] });
  http.read.mockResolvedValue({
    confirmation: {
      ...row,
      subject: row.principal,
      status: "approved",
      approved_at: row.created_at,
      approved_until: row.expires_at,
    },
  });
  mount();
  await openReview();
  // Keep the post-decision live read pending: the notice must describe the
  // accepted approval, never infer that the caller's operation executed.
  http.get.mockImplementation(() => new Promise(() => {}));
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I reviewed this identity/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve one retry" }),
  );
  expect(
    await screen.findByText(
      /Approval recorded\. Waiting for the original caller to retry/,
    ),
  ).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("labels approved rows as awaiting caller retry and keeps Assistant decisions in the original conversation", async () => {
  http.get.mockResolvedValue({
    confirmations: [
      { ...payload(), state: "approved", decided_at: payload().created_at },
    ],
  });
  mount();
  expect(
    await screen.findByText(
      /Approved — waiting for the original caller to retry/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /Assistant action confirmations stay in the original conversation/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Review decision" }),
  ).toBeDisabled();
  expect(http.read).not.toHaveBeenCalled();
});
