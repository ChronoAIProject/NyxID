import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  AssistantKeyCreateDialog,
  type AssistantKeyCreateParams,
} from "./assistant-key-create-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number) {
      super(`HTTP ${String(status)}`);
      this.status = status;
    }
  },
}));

const PARAMS = {
  name: "coding-agent",
  platform: "codex",
  allowedServiceIds: ["service-alpha"],
} as const;

function keySnapshot(overrides: Record<string, unknown> = {}) {
  return {
    id: "key-created",
    name: "coding-agent",
    platform: "codex",
    scopes: "proxy",
    is_active: true,
    allowed_service_ids: ["service-alpha"],
    allowed_node_ids: [],
    allow_all_services: false,
    allow_all_nodes: false,
    key_prefix: "nyxid_ag_safe_prefix",
    ...overrides,
  };
}

function installSuccessfulReads(overrides: Record<string, unknown> = {}) {
  mockGet.mockImplementation((path: string) => {
    if (path === "/keys/service-alpha") {
      return Promise.resolve({
        id: "service-alpha",
        is_active: true,
        credential_source: { type: "personal" },
      });
    }
    if (path === "/api-keys/key-created") {
      return Promise.resolve(keySnapshot(overrides));
    }
    return Promise.reject(new Error(`unexpected GET ${path}`));
  });
}

function renderDialog(
  params: AssistantKeyCreateParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyCreateDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-alpha"
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

describe("AssistantKeyCreateDialog", () => {
  it("fences double submission and reports only after exact read-back and secret acknowledgement", async () => {
    installSuccessfulReads();
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: false,
      fullKey: "nyxid_ag_one_time_secret",
    });
    const { onComplete } = renderDialog();

    const create = screen.getByRole("button", { name: "Create key" });
    fireEvent.click(create);
    fireEvent.click(create);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockGet).toHaveBeenNthCalledWith(1, "/keys/service-alpha");
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/key-create", {
      actionRequestId: "action-alpha",
      name: "coding-agent",
      platform: "codex",
      allowedServiceIds: ["service-alpha"],
    });
    expect(mockGet).toHaveBeenNthCalledWith(2, "/api-keys/key-created");

    expect(
      await screen.findByText("nyxid_ag_one_time_secret"),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Exact least-scope access verified."),
    ).toBeInTheDocument();
    const finish = screen.getByRole("button", {
      name: "I have saved it",
    });
    expect(finish).toBeDisabled();
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: "I saved this key in a secure location.",
      }),
    );
    expect(finish).toBeEnabled();
    await userEvent.click(finish);
    expect(onComplete).toHaveBeenCalledWith("key-created");
    expect(onComplete).not.toHaveBeenCalledWith(
      expect.stringContaining("one_time_secret"),
    );
  });

  it("rejects empty and duplicate service sets before any provider read", async () => {
    for (const allowedServiceIds of [[], ["service-alpha", "service-alpha"]]) {
      const { unmount } = renderDialog({ ...PARAMS, allowedServiceIds });
      await userEvent.click(screen.getByRole("button", { name: "Create key" }));
      expect(await screen.findByRole("alert")).toBeInTheDocument();
      expect(mockGet).not.toHaveBeenCalled();
      expect(mockPost).not.toHaveBeenCalled();
      unmount();
      mockGet.mockClear();
      mockPost.mockClear();
    }
  });

  it("rejects unknown and cross-owner service reads before mutation", async () => {
    const failures = [
      () =>
        Promise.resolve({
          id: "different-service",
          is_active: true,
          credential_source: { type: "personal" },
        }),
      () =>
        Promise.resolve({
          id: "service-alpha",
          is_active: true,
          credential_source: {
            type: "org",
            org_id: "org-alpha",
            role: "member",
            allowed: true,
          },
        }),
      () =>
        Promise.reject(
          new ApiError(404, {
            error: "not_found",
            error_code: 404,
            message: "Service not found",
          }),
        ),
    ];
    for (const failure of failures) {
      mockGet.mockImplementationOnce(failure);
      const { unmount } = renderDialog();
      await userEvent.click(screen.getByRole("button", { name: "Create key" }));
      expect(await screen.findByRole("alert")).toBeInTheDocument();
      expect(mockPost).not.toHaveBeenCalled();
      unmount();
      mockGet.mockReset();
      mockPost.mockReset();
    }
  });

  it("keeps the safe report blocked when authoritative read-back is widened", async () => {
    installSuccessfulReads({ allow_all_services: true });
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: false,
      fullKey: "nyxid_ag_one_time_secret",
    });
    const { onComplete } = renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(
      await screen.findByText("nyxid_ag_one_time_secret"),
    ).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: "I saved this key in a secure location.",
      }),
    );
    expect(
      screen.getByRole("button", { name: "I have saved it" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Retry verification" }),
    ).toBeEnabled();
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("rejects secret-bearing exact read-back evidence", async () => {
    installSuccessfulReads({ full_key: "nyxid_ag_should_not_be_here" });
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: false,
      fullKey: "nyxid_ag_one_time_secret",
    });
    renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(
      screen.getByRole("button", { name: "I have saved it" }),
    ).toBeDisabled();
  });

  it("replays a verified safe key receipt without pretending the secret is recoverable", async () => {
    installSuccessfulReads();
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: true,
    });
    const { onComplete } = renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(
      await screen.findByText(/one-time secret is no longer available/i),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Exact least-scope access verified."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "Report existing key" }),
    );
    expect(onComplete).toHaveBeenCalledWith("key-created");
  });

  it("recovers a durable replay after an allowed service is deactivated", async () => {
    installSuccessfulReads();
    mockGet.mockImplementation((path: string) => {
      if (path === "/keys/service-alpha") {
        return Promise.resolve({
          id: "service-alpha",
          is_active: false,
          credential_source: { type: "personal" },
        });
      }
      if (path === "/api-keys/key-created") {
        return Promise.resolve(keySnapshot());
      }
      return Promise.reject(new Error(`unexpected GET ${path}`));
    });
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: true,
    });
    const { onComplete } = renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(
      await screen.findByText("Exact least-scope access verified."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Report existing key" }),
    );
    expect(onComplete).toHaveBeenCalledWith("key-created");
  });

  it("rejects a replay response that includes secret material", async () => {
    mockGet.mockResolvedValue({
      id: "service-alpha",
      is_active: true,
      credential_source: { type: "personal" },
    });
    mockPost.mockResolvedValue({
      resource: { keyId: "key-created" },
      replayed: true,
      fullKey: "nyxid_ag_should_not_exist",
    });
    renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(
      screen.queryByText("nyxid_ag_should_not_exist"),
    ).not.toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledTimes(1);
  });
});
