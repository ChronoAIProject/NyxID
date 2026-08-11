import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AssistantKeyRotateDialog,
  type AssistantKeyRotateParams,
} from "./assistant-key-rotate-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {},
}));

const PARAMS = {
  keyId: "00000000-0000-4000-8000-000000000001",
} as const;
const REPLACEMENT_ID = "00000000-0000-4000-8000-000000000002";
const REQUESTED_AT = "2026-08-11T08:00:00Z";

function replacementSnapshot(overrides: Record<string, unknown> = {}) {
  return {
    id: REPLACEMENT_ID,
    name: "coding-agent",
    platform: "codex",
    scopes: "proxy",
    is_active: true,
    created_at: "2026-08-11T08:00:01Z",
    rotation_predecessor_id: PARAMS.keyId,
    state_version: 1,
    updated_at: "2026-08-11T08:00:01Z",
    allowed_service_ids: ["service-alpha"],
    allowed_node_ids: [],
    allow_all_services: false,
    allow_all_nodes: false,
    ...overrides,
  };
}

function createdEffect(overrides: Record<string, unknown> = {}) {
  return {
    resource: { keyId: REPLACEMENT_ID },
    replayed: false,
    requestedAt: REQUESTED_AT,
    fullKey: "nyxid_ag_one_time_replacement",
    ...overrides,
  };
}

function renderDialog(
  params: AssistantKeyRotateParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyRotateDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-rotate-alpha"
      params={params}
      onComplete={onComplete}
    />,
  );
  return { ...rendered, onComplete, onOpenChange };
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantKeyRotateDialog", () => {
  it("fences double submission and reports only an exactly verified replacement id", async () => {
    mockPost.mockResolvedValue(createdEffect());
    mockGet.mockResolvedValue(replacementSnapshot());
    const { onComplete } = renderDialog();

    const rotate = screen.getByRole("button", { name: "Rotate key" });
    fireEvent.click(rotate);
    fireEvent.click(rotate);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/key-rotate", {
      actionRequestId: "action-rotate-alpha",
      keyId: PARAMS.keyId,
    });
    expect(mockGet).toHaveBeenCalledWith(`/api-keys/${REPLACEMENT_ID}`);
    expect(
      await screen.findByText("nyxid_ag_one_time_replacement"),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Exact rotation lineage verified."),
    ).toBeInTheDocument();

    const finish = screen.getByRole("button", { name: "I have saved it" });
    expect(finish).toBeDisabled();
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: "I saved this replacement key in a secure location.",
      }),
    );
    await userEvent.click(finish);
    expect(onComplete).toHaveBeenCalledWith(REPLACEMENT_ID);
    expect(onComplete).not.toHaveBeenCalledWith(
      expect.stringContaining("one_time_replacement"),
    );
  }, 15_000);

  it("replays the same verified replacement without replaying secret material", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: REPLACEMENT_ID },
      replayed: true,
      requestedAt: REQUESTED_AT,
    });
    mockGet.mockResolvedValue(replacementSnapshot());
    const { onComplete } = renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(
      await screen.findByText(
        /one-time replacement secret is no longer available/i,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Report replacement key" }),
    );
    expect(onComplete).toHaveBeenCalledWith(REPLACEMENT_ID);
  });

  it("fails closed on mismatched, stale, zero-version, inactive, and secret-bearing read-back", async () => {
    const invalidSnapshots = [
      replacementSnapshot({ id: "00000000-0000-4000-8000-000000000009" }),
      replacementSnapshot({ rotation_predecessor_id: REPLACEMENT_ID }),
      replacementSnapshot({ rotation_predecessor_id: null }),
      replacementSnapshot({ state_version: 0 }),
      replacementSnapshot({ is_active: false }),
      replacementSnapshot({ created_at: "2026-08-11T07:59:59Z" }),
      replacementSnapshot({ updated_at: "2026-08-11T07:59:59Z" }),
      replacementSnapshot({ full_key: "nyxid_ag_must_not_escape" }),
      replacementSnapshot({ ignored: { Authorization: "Bearer hidden" } }),
    ];

    for (const snapshot of invalidSnapshots) {
      mockPost.mockResolvedValueOnce(createdEffect());
      mockGet.mockResolvedValueOnce(snapshot);
      const { unmount, onComplete } = renderDialog();
      await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
      expect(await screen.findByRole("alert")).toBeInTheDocument();
      await userEvent.click(
        screen.getByRole("checkbox", {
          name: "I saved this replacement key in a secure location.",
        }),
      );
      expect(
        screen.getByRole("button", { name: "I have saved it" }),
      ).toBeDisabled();
      expect(onComplete).not.toHaveBeenCalled();
      unmount();
    }
  }, 30_000);

  it("accepts additive safe read-back fields", async () => {
    mockPost.mockResolvedValue(createdEffect());
    mockGet.mockResolvedValue(
      replacementSnapshot({
        future_safe_field: { authority_source: "api_keys" },
      }),
    );
    renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(
      await screen.findByText("Exact rotation lineage verified."),
    ).toBeInTheDocument();
  });

  it("rejects a replay response widened with one-time key material", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: REPLACEMENT_ID },
      replayed: true,
      requestedAt: REQUESTED_AT,
      fullKey: "nyxid_ag_must_not_replay",
    });
    renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(
      screen.queryByText("nyxid_ag_must_not_replay"),
    ).not.toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();
  });

  it("rejects malformed predecessor identities before mutation", async () => {
    renderDialog({ keyId: "invalid/key" });

    await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
    expect(mockGet).not.toHaveBeenCalled();
  });
});
