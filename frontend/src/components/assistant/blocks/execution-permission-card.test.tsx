import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ExecutionPermissionCard } from "./execution-permission-card";
import { ApprovalCard } from "./approval-card";
import type { ExecutionProposal } from "@/schemas/execution-proposal";
import { api } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn(), post: vi.fn() } }));
const proposal: ExecutionProposal = {
  version: 1,
  supported: true,
  revision: 2,
  state: "pending",
  can_decide: true,
  review_path: "/assistant/approvals",
  grant_id: null,
  failure_code: null,
  review: {
    agent: "Researcher",
    connection: "work-account · your connection · 12345678",
    operation: "POST /notes",
    audience: "This group and its current participants",
    scope: "Only these inputs",
    inputs: '{"body":{"title":"Note"}}',
    existing_permission: "No complete permission",
    added_permission: "personal service approval",
    reusable_unavailable: null,
  },
  choices: [
    {
      id: "operation",
      digest: "operation-digest",
      label: "This operation",
      valid_until: "2099-01-01T00:00:00Z",
    },
    {
      id: "task",
      digest: "task-digest",
      label: "This task",
      valid_until: "2099-01-02T00:00:00Z",
    },
    {
      id: "window",
      digest: "window-digest",
      label: "For one hour",
      valid_until: "2099-01-01T01:00:00Z",
    },
    {
      id: "persistent",
      digest: "persistent-digest",
      label: "Until revoked or this account binding changes",
      valid_until: null,
    },
  ],
};
let client: QueryClient;
beforeEach(() => {
  vi.clearAllMocks();
  client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity },
      mutations: { retry: false },
    },
  });
  vi.mocked(api.get).mockResolvedValue({ execution_proposal: proposal });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});
function card(value = proposal) {
  return render(
    <QueryClientProvider client={client}>
      <ExecutionPermissionCard requestId="request" proposal={value} />
    </QueryClientProvider>,
  );
}
it("defaults to one operation and sends only the bound human choice", async () => {
  vi.mocked(api.post).mockResolvedValue({
    execution_proposal: {
      ...proposal,
      state: "queued",
      can_decide: false,
      grant_id: "grant",
    },
  });
  card();
  expect(screen.getByRole("radio", { name: /This operation/ })).toBeChecked();
  expect(screen.getByText(proposal.review!.connection)).toBeInTheDocument();
  await act(async () => {
    fireEvent.click(
      screen.getByRole("button", { name: "Allow this operation" }),
    );
  });
  expect(api.post).toHaveBeenCalledWith(
    "/approvals/requests/request/execution-decision",
    expect.objectContaining({
      approved: true,
      expected_revision: 2,
      choice_id: "operation",
      choice_digest: "operation-digest",
      idempotency_key: expect.any(String),
    }),
  );
  expect(
    screen.getByText("Permission saved · operation queued"),
  ).toBeInTheDocument();
  expect(screen.queryByText("Operation completed")).not.toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: /Inspect or revoke/ }),
  ).toHaveAttribute("href", "/approvals/grants");
});
it("selects an enforceable persistent choice without changing scope", async () => {
  vi.mocked(api.post).mockResolvedValue({
    execution_proposal: { ...proposal, state: "queued", can_decide: false },
  });
  card();
  fireEvent.click(screen.getByRole("radio", { name: /Until revoked/ }));
  await act(async () => {
    fireEvent.click(
      screen.getByRole("button", { name: "Allow selected permission" }),
    );
  });
  expect(api.post).toHaveBeenCalledWith(
    expect.any(String),
    expect.objectContaining({
      choice_id: "persistent",
      choice_digest: "persistent-digest",
    }),
  );
  expect(vi.mocked(api.post).mock.calls[0]?.[1]).not.toHaveProperty("scope");
});

it("requires an explicit choice for private conversations and submits only its bound identity", async () => {
  const value: ExecutionProposal = {
    ...proposal,
    version: 3,
    choices: [
      ...proposal.choices.map((choice) => ({
        ...choice,
        context_scope: "current_conversation" as const,
      })),
      {
        id: "persistent_private_conversations",
        digest: "private-conversations-digest",
        label: "This agent, this connection and the displayed operation/inputs, across my private conversations, until revoked",
        valid_until: null,
        context_scope: "private_conversations",
      },
    ],
  };
  vi.mocked(api.post).mockResolvedValue({
    execution_proposal: { ...value, state: "queued", can_decide: false },
  });
  card(value);
  expect(screen.getByRole("radio", { name: /This operation/ })).toBeChecked();
  expect(screen.getByText("Applies to: Only this conversation")).toBeInTheDocument();
  const across = screen.getByRole("radio", { name: /across my private conversations/ });
  expect(across).not.toBeChecked();
  fireEvent.click(across);
  expect(screen.getByText("Applies to: Across my private conversations")).toBeInTheDocument();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Allow selected permission" }));
  });
  expect(api.post).toHaveBeenCalledWith(
    "/approvals/requests/request/execution-decision",
    {
      approved: true,
      expected_revision: 2,
      choice_id: "persistent_private_conversations",
      choice_digest: "private-conversations-digest",
      idempotency_key: expect.any(String),
    },
  );
});
it("reloads a lost decision response without blindly retrying the effect", async () => {
  vi.mocked(api.post).mockRejectedValue(new Error("Response lost"));
  vi.mocked(api.get).mockResolvedValue({
    execution_proposal: { ...proposal, state: "completed", can_decide: false },
  });
  card();
  fireEvent.click(screen.getByRole("button", { name: "Allow this operation" }));
  await waitFor(() =>
    expect(screen.getByText("Operation completed")).toBeInTheDocument(),
  );
  expect(api.post).toHaveBeenCalledTimes(1);
  expect(
    screen.queryByRole("button", { name: /Allow/ }),
  ).not.toBeInTheDocument();
});
it("refreshes a pending card decided on another device without sending a decision", async () => {
  vi.useFakeTimers();
  card();
  expect(screen.getByText("Execution permission required")).toBeInTheDocument();
  vi.mocked(api.get).mockResolvedValue({
    execution_proposal: { ...proposal, state: "completed", can_decide: false },
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2100);
  });
  expect(screen.getByText("Operation completed")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Allow/ })).not.toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});
it("shows expired and unknown outcomes without an automatic resend action", () => {
  const first = card({ ...proposal, state: "expired", can_decide: false });
  expect(
    screen.getByText(/explicitly request the task again/),
  ).toBeInTheDocument();
  first.unmount();
  client.clear();
  card({ ...proposal, state: "unknown", can_decide: false });
  expect(screen.getByText(/outcome unknown/)).toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});
it("shows the new card through the legacy host without using its Allow callback", async () => {
  const legacy = vi.fn();
  vi.mocked(api.post).mockResolvedValue({
    execution_proposal: { ...proposal, state: "denied", can_decide: false },
  });
  render(
    <QueryClientProvider client={client}>
      <ApprovalCard
        block={{
          type: "approval_card",
          block_id: "b",
          approval_request_id: "request",
          body: "Review",
          service_slug: "notes",
          agent_key_prefix: "key",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: "2099-01-01T00:00:00Z",
          decision: null,
          decision_channel: null,
          execution_proposal: proposal,
        }}
        onDecide={legacy}
      />
    </QueryClientProvider>,
  );
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  });
  expect(legacy).not.toHaveBeenCalled();
  expect(api.post).toHaveBeenCalledWith(
    expect.stringContaining("execution-decision"),
    expect.objectContaining({
      approved: false,
      choice_id: null,
      choice_digest: null,
    }),
  );
});
it("does not offer unsupported reusable choices or interpret an unknown version", () => {
  const first = card({
    ...proposal,
    choices: [proposal.choices[0]!],
    review: {
      ...proposal.review!,
      reusable_unavailable: "Only this operation is representable",
    },
  });
  expect(screen.getAllByRole("radio")).toHaveLength(1);
  expect(
    screen.getByText("Only this operation is representable"),
  ).toBeInTheDocument();
  first.unmount();
  client.clear();
  card({
    ...proposal,
    version: 99,
    supported: false,
    state: "unsupported",
    can_decide: false,
    choices: [],
    review: null,
  });
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});


it("renders the requester's retained result as plain text without claiming another send", () => {
  card({...proposal, state: "completed", can_decide: false,
    result_text: 'HTTP 200: <img src=x onerror=alert(1)>',
  });
  expect(screen.getByText('HTTP 200: <img src=x onerror=alert(1)>')).toBeInTheDocument();
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});

it("shows an external v2 request without inventing a task choice", () => {
  card({
    ...proposal,
    version: 2,
    source: "API key request",
    choices: proposal.choices.filter((choice) => choice.id !== "task"),
    review: {
      ...proposal.review!,
      audience: "Only this authenticated caller",
      reusable_unavailable: "Task is unavailable: this request has no server-owned task",
    },
  });
  expect(screen.getByText("Source:")).toBeInTheDocument();
  expect(screen.getByText("API key request")).toBeInTheDocument();
  expect(screen.queryByRole("radio", { name: /This task/ })).not.toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /This operation/ })).toBeChecked();
  expect(screen.getByText(/no server-owned task/)).toBeInTheDocument();
});

it("keeps login-child review Operation-only and explains external expiry", () => {
  const first = card({
    ...proposal, version: 2, source: "API key request",
    choices: [proposal.choices[0]!],
    review: {...proposal.review!, reusable_unavailable: "Login-child credentials support only this operation"},
  });
  expect(screen.getAllByRole("radio")).toHaveLength(1);
  expect(screen.getByText(/Login-child credentials/)).toBeInTheDocument();
  first.unmount(); client.clear();
  card({...proposal, version: 2, state: "expired", can_decide: false});
  expect(screen.getByText(/use a new request context/)).toBeInTheDocument();
  expect(screen.queryByText(/original conversation/)).not.toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});
