import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import type {
  AddKeyDialogCompletion,
  AuthorizationAttempt,
} from "@/components/dashboard/add-key-dialog";
import {
  CONNECT_WATCH_ACTIVITY_GRACE_MS,
  CONNECT_WATCH_BASE_MS,
  markChatActivity,
  resetChatActivityForTests,
} from "@/lib/assistant/connect-watch";
import type { ActionReport } from "@/schemas/assistant-actions";
import { usePendingConnectStore } from "@/stores/pending-connect-store";
import type { ActionCardContentBlock } from "@/types/assistant";
import { ActionCard } from "./action-card";

const { mockGet } = vi.hoisted(() => ({ mockGet: vi.fn() }));

// The pending-authorization store outlives any one card by design, so it also
// outlives any one test. Reset it or a stranded attempt leaks forward.
beforeEach(() => {
  mockGet.mockReset();
  usePendingConnectStore.setState({ attempts: {} });
});

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet },
  ApiError: class ApiError extends Error {
    readonly status: number;

    constructor(status: number) {
      super("API request failed");
      this.status = status;
    }
  },
}));

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

function renderCard(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(ui, {
    wrapper: ({ children }: { readonly children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
  });
}

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    onOpenChange,
    prefillSlug,
    prefillIncludeAllCatalog,
    prefillCustom,
    prefillScopes,
    reconnectKey,
    onSuccess,
    onAuthorizationPending,
    onAuthorizationAborted,
    launch,
    flow,
    onPopupViewResult,
  }: {
    readonly open: boolean;
    readonly onOpenChange: (open: boolean) => void;
    readonly prefillSlug?: string;
    readonly prefillIncludeAllCatalog?: boolean;
    readonly prefillCustom?: { readonly name?: string };
    readonly prefillScopes?: readonly string[];
    readonly reconnectKey?: {
      readonly id: string;
      readonly last_authorized_at?: string | null;
    } | null;
    readonly onSuccess?: (result: AddKeyDialogCompletion) => void;
    readonly onAuthorizationPending?: (attempt: AuthorizationAttempt) => void;
    readonly onAuthorizationAborted?: (attemptId: string) => void;
    readonly launch?: string;
    readonly flow?: string;
    readonly onPopupViewResult?: (keyId: string) => boolean;
  }) =>
    open ? (
      <div
        role="dialog"
        data-prefill={prefillSlug ?? prefillCustom?.name ?? ""}
        data-prefill-include-all={String(prefillIncludeAllCatalog ?? false)}
        data-prefill-scopes={prefillScopes?.join(",") ?? ""}
        data-reconnect-key={reconnectKey?.id ?? ""}
        data-launch={launch ?? ""}
        data-flow={flow ?? ""}
      >
        <button
          type="button"
          onClick={() =>
            onPopupViewResult?.(reconnectKey?.id ?? "key-pending-1")
          }
        >
          Return from popup
        </button>
        <button
          type="button"
          onClick={() =>
            onSuccess?.({
              userServiceId:
                reconnectKey?.id ?? "00000000-0000-4000-8000-000000000123",
            })
          }
        >
          Finish mock connection
        </button>
        <button
          type="button"
          onClick={() => onSuccess?.({ userServiceId: "" })}
        >
          Finish without service id
        </button>
        <button
          type="button"
          onClick={() => onSuccess?.({ userServiceId: "   " })}
        >
          Finish with whitespace service id
        </button>
        <button
          type="button"
          onClick={() =>
            onAuthorizationPending?.({
              keyId: reconnectKey?.id ?? "key-pending-1",
              attemptId: "attempt-pending-1",
              previousAuthorizationAt: reconnectKey?.last_authorized_at,
            })
          }
        >
          Hand off to provider
        </button>
        <button
          type="button"
          onClick={() => {
            onAuthorizationAborted?.("attempt-pending-1");
            onOpenChange(false);
          }}
        >
          Dismiss mock connection
        </button>
      </div>
    ) : null,
}));

vi.mock("@/components/assistant/assistant-key-create-dialog", () => ({
  AssistantKeyCreateDialog: ({
    open,
    actionRequestId,
    params,
    onComplete,
  }: {
    readonly open: boolean;
    readonly actionRequestId: string;
    readonly params: {
      readonly name: string;
      readonly platform: string;
      readonly allowedServiceIds: readonly string[];
    };
    readonly onComplete: (keyId: string) => void;
  }) =>
    open ? (
      <div
        role="dialog"
        data-action-request-id={actionRequestId}
        data-name={params.name}
        data-platform={params.platform}
        data-service-ids={params.allowedServiceIds.join(",")}
      >
        <button type="button" onClick={() => onComplete("key-created")}>
          Finish mock key creation
        </button>
      </div>
    ) : null,
}));

vi.mock("@/components/assistant/assistant-key-rotate-dialog", () => ({
  AssistantKeyRotateDialog: ({
    open,
    actionRequestId,
    params,
    onComplete,
  }: {
    readonly open: boolean;
    readonly actionRequestId: string;
    readonly params: { readonly keyId: string };
    readonly onComplete: (keyId: string) => void;
  }) =>
    open ? (
      <div
        role="dialog"
        data-action-request-id={actionRequestId}
        data-key-id={params.keyId}
      >
        <button type="button" onClick={() => onComplete("key-replacement")}>
          Finish mock key rotation
        </button>
      </div>
    ) : null,
}));

function catalogBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-1",
    action: "service.connect",
    action_request_id: "act-1",
    origin_turn_id: "turn-origin-1",
    task_id: "task-1",
    step_id: "step-1",
    params: {
      variant: "catalog",
      service_slug: "api-github",
      requested_scopes: ["repo"],
      via_node_id: "node-1",
      target_org_id: "org-1",
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

function keyCreateBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-key-create",
    action: "key.create",
    action_request_id: "act-key-create",
    origin_turn_id: "turn-origin-1",
    task_id: "task-1",
    step_id: "step-1",
    params: {
      variant: "key_create",
      name: "coding-agent",
      platform: "codex",
      allowed_service_ids: ["service-alpha"],
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

function reauthorizeBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-reauthorize",
    action: "service.reauthorize",
    action_request_id: "act-reauthorize",
    origin_turn_id: "turn-origin-1",
    task_id: "task-1",
    step_id: "step-1",
    params: {
      variant: "service_reauthorize",
      user_service_id: "service-existing",
      requested_scopes: ["repo", "workflow"],
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

function reauthorizationKey(
  overrides: Readonly<Record<string, unknown>> = {},
): Readonly<Record<string, unknown>> {
  return {
    id: "service-existing",
    slug: "github-user",
    label: "GitHub",
    api_key_id: "credential-existing",
    credential_missing: false,
    credential_type: "oauth2",
    auth_method: "oauth2",
    is_active: true,
    auto_connected: false,
    catalog_service_slug: "github",
    credential_source: { type: "personal" },
    status: "active",
    last_authorized_at: "2026-08-17T00:00:00Z",
    ...overrides,
  };
}

function reauthorizationCatalog(
  overrides: Readonly<Record<string, unknown>> = {},
) {
  return {
    entries: [
      {
        slug: "github",
        provider_type: "oauth2",
        provider_config_id: "provider-github",
        device_code_format: null,
        ...overrides,
      },
      {
        slug: "plain-api",
        provider_type: "api_key",
        provider_config_id: null,
        device_code_format: null,
      },
    ],
  };
}

function mockReauthorizationReads(
  key: Readonly<Record<string, unknown>> = reauthorizationKey(),
  catalog = reauthorizationCatalog(),
) {
  mockGet.mockImplementation((path: string) => {
    if (path === "/keys/service-existing") return Promise.resolve(key);
    if (path === "/catalog?include_all=true") {
      return Promise.resolve(catalog);
    }
    return Promise.reject(new Error(`Unexpected read: ${path}`));
  });
}

function keyRotateBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-key-rotate",
    action: "key.rotate",
    action_request_id: "act-key-rotate",
    origin_turn_id: "turn-origin-1",
    task_id: "task-1",
    step_id: "step-1",
    params: {
      variant: "key_rotate",
      key_id: "key-predecessor",
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

function expectNoBlueAccent(card: HTMLElement | null) {
  const classNames = [card, ...(card?.querySelectorAll("[class]") ?? [])]
    .map((element) => element?.getAttribute("class") ?? "")
    .join(" ");
  expect(classNames).not.toMatch(
    /(?:^|\s)(?:[a-z-]+:)*(?:bg|border|text|ring|fill|stroke)-(?:info|blue|sky|cyan|indigo)(?:[\w./-]*)/,
  );
}

describe("ActionCard", () => {
  it("opens exact reconnect mode with every requested scope and reports only the service id", async () => {
    mockReauthorizationReads();
    const onProgress = vi.fn();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={reauthorizeBlock()}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Re-authorize service" }),
    ).toBeInTheDocument();
    expect(screen.getByText("service-existing")).toBeInTheDocument();
    expect(screen.getByText("workflow")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Re-authorize" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-reauthorize", true);
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveAttribute("data-reconnect-key", "service-existing");
    expect(dialog).toHaveAttribute("data-prefill-scopes", "repo,workflow");
    expect(dialog).toHaveAttribute("data-prefill-include-all", "true");
    expect(dialog).toHaveAttribute("data-launch", "popup");
    expect(dialog).toHaveAttribute("data-flow", "cc");
    expect(mockGet).toHaveBeenCalledWith("/keys/service-existing");
    expect(mockGet).toHaveBeenCalledWith("/catalog?include_all=true");

    await userEvent.click(
      screen.getByRole("button", { name: "Finish mock connection" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-reauthorize",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: { userService: { userServiceId: "service-existing" } },
    });
  });

  it("blocks non-OAuth services before opening a reauthorization dialog", async () => {
    mockReauthorizationReads(
      reauthorizationKey({
        credential_type: "api_key",
        auth_method: "header",
      }),
    );
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={reauthorizeBlock()}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Re-authorize" }));
    await waitFor(() => {
      expect(onBlock).toHaveBeenCalledWith(
        "action-card-reauthorize",
        expect.stringContaining("does not support browser re-authorization"),
      );
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(onResolve).not.toHaveBeenCalled();
    expect(mockGet).not.toHaveBeenCalledWith("/catalog?include_all=true");
  });

  it("blocks OpenAI-format device authorization instead of dropping requested scopes", async () => {
    mockReauthorizationReads(
      reauthorizationKey({ catalog_service_slug: "codex" }),
      reauthorizationCatalog({
        slug: "codex",
        provider_type: "device_code",
        provider_config_id: "provider-codex",
        device_code_format: "openai",
      }),
    );
    const onBlock = vi.fn();
    renderCard(
      <ActionCard
        block={reauthorizeBlock()}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Re-authorize" }));
    await waitFor(() => {
      expect(onBlock).toHaveBeenCalledWith(
        "action-card-reauthorize",
        expect.stringContaining("does not accept requested scope changes"),
      );
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("waits for a fresh authorization timestamp before auto-completing reauthorization", async () => {
    let keyReads = 0;
    mockGet.mockImplementation((path: string) => {
      if (path === "/keys/service-existing") {
        keyReads += 1;
        return Promise.resolve(
          keyReads === 1
            ? reauthorizationKey()
            : reauthorizationKey({
                last_authorized_at: "2026-08-17T00:01:00Z",
              }),
        );
      }
      if (path === "/catalog?include_all=true") {
        return Promise.resolve(reauthorizationCatalog());
      }
      return Promise.reject(new Error(`Unexpected read: ${path}`));
    });
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={reauthorizeBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Re-authorize" }));
    await userEvent.click(
      await screen.findByRole("button", { name: "Hand off to provider" }),
    );

    await waitFor(() => {
      expect(onResolve).toHaveBeenCalledWith({
        actionRequestId: "act-reauthorize",
        originTurnId: "turn-origin-1",
        disposition: "completed",
        resource: { userService: { userServiceId: "service-existing" } },
      });
    });
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it("opens the key.create journey and reports only the safe key identity", async () => {
    const onProgress = vi.fn();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={keyCreateBlock()}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Create API key" }),
    ).toBeInTheDocument();
    expect(screen.getByText("service-alpha")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create key" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-key-create", true);
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-action-request-id",
      "act-key-create",
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Finish mock key creation" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-key-create",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: { key: { keyId: "key-created" } },
    });
  });

  it("opens the key.rotate journey and reports only the replacement identity", async () => {
    const onProgress = vi.fn();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={keyRotateBlock()}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Rotate API key" }),
    ).toBeInTheDocument();
    expect(screen.getByText("key-predecessor")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-key-rotate", true);
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-action-request-id",
      "act-key-rotate",
    );
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-key-id",
      "key-predecessor",
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Finish mock key rotation" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-key-rotate",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: { key: { keyId: "key-replacement" } },
    });
  });

  it("renders owned consent copy and opens the prefilled connect journey", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Connect GitHub" }),
    ).toBeInTheDocument();
    const card = screen
      .getByRole("heading", { name: "Connect GitHub" })
      .closest("section");
    expect(card).toHaveClass("border-border", "bg-card");
    expect(card?.className).not.toContain("warning");
    expect(card?.querySelector('[class*="warning"]')).toBeNull();
    expect(card?.firstElementChild).toHaveClass("flex", "items-start");
    expect(screen.getByText("repo")).toBeInTheDocument();
    expect(screen.getByText("Node node-1")).toBeInTheDocument();
    expect(screen.getByText("Org org-1")).toBeInTheDocument();
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-prefill",
      "api-github",
    );
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-prefill-include-all",
      "true",
    );

    await user.click(
      screen.getByRole("button", { name: "Finish mock connection" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: {
        userService: {
          userServiceId: "00000000-0000-4000-8000-000000000123",
        },
      },
    });
  });

  it("hands the connect journey to the managed popup, not the chat tab", async () => {
    // Provider consent pages cannot be iframed, so the popup is the only
    // handoff that keeps the conversation alive underneath it. Without these
    // two props the dialog silently takes the legacy path: a second click on
    // a `target="_blank"` link, and a callback that redirects the chat tab to
    // the key page.
    const user = userEvent.setup();
    renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("data-launch", "popup");
    expect(dialog).toHaveAttribute("data-flow", "cc");
  });

  it("closes the dialog on return from the popup without abandoning the connection", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });
    const { rerender } = renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Hand off to provider" }),
    );
    await user.click(screen.getByRole("button", { name: "Return from popup" }));

    // Back in the transcript, and still owned by the card's watch — the
    // outcome belongs in the conversation, not on a detour to the keys page.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onProgress).not.toHaveBeenCalledWith("action-card-1", false);
  });

  it("rolls a rejected completed report out of its busy projection", async () => {
    // The connect journey projects the card to in-progress before the report
    // is delivered. If delivery dies, the card must return to its actionable
    // state — not sit at "Connecting" with every control disabled and no
    // visible recovery. (The page toasts the failure; this is the card's
    // half of the contract.)
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onResolve = vi.fn().mockRejectedValue(new Error("delivery failed"));
    renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    await user.click(
      screen.getByRole("button", { name: "Finish mock connection" }),
    );

    await waitFor(() => {
      expect(onProgress).toHaveBeenCalledWith("action-card-1", false);
    });
    // Retryable: the card's own controls are live again.
    expect(screen.getByRole("button", { name: "Decline" })).toBeEnabled();
  });

  it("never renders a colored top accent rail", () => {
    const { rerender } = renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    function expectNoRail() {
      const card = screen.getByRole("heading").closest("section");
      const rootChildren = [...(card?.children ?? [])];
      expect(
        rootChildren.some((child) => {
          const classes = child.getAttribute("class") ?? "";
          return (
            /h-\[(2|3|4)px\]/.test(classes) &&
            /bg-(nyx-secondary-400|destructive)/.test(classes)
          );
        }),
      ).toBe(false);
      expect(
        rootChildren.some((child) =>
          child.classList.contains("bg-nyx-secondary-400"),
        ),
      ).toBe(false);
    }

    expectNoRail();
    rerender(
      <ActionCard
        block={catalogBlock({
          action: "future.action",
          params: { variant: "unknown" },
          status: "unsupported",
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );
    expectNoRail();
  });

  it("uses purple interaction accents and neutral reference chips without blue", () => {
    const { rerender } = renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    function expectPurpleAndNeutralPalette(status: "pending" | "in_progress") {
      const statusLabel =
        status === "pending" ? "Action required" : "In progress";
      const card = screen.getByRole("heading").closest("section");
      expectNoBlueAccent(card);
      expect(screen.getByText(statusLabel).closest("div")).toHaveClass(
        "text-nyx-secondary-400",
      );
      expect(screen.getByText("Node node-1").closest("div")).toHaveClass(
        "bg-muted",
        "text-muted-foreground",
      );
      expect(card?.querySelector("svg.text-nyx-secondary-400")).not.toBeNull();
    }

    expectPurpleAndNeutralPalette("pending");
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );
    expectPurpleAndNeutralPalette("in_progress");
  });

  it("treats modal dismissal as pending and decline as an explicit report", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    const { rerender } = renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );
    expect(onProgress).toHaveBeenLastCalledWith("action-card-1", false);
    expect(onResolve).not.toHaveBeenCalled();

    rerender(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });
  });

  it("keeps the whole card on a terminal outcome and drops only its controls", () => {
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    const { rerender } = renderCard(
      <ActionCard
        block={catalogBlock({
          status: "completed",
          outcome_note: "Connected. The assistant can use this service now.",
        })}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    // The service, scopes, and routing the user consented to survive the
    // verdict; only the CTA row and the connect dialog go away.
    expect(
      screen.getByRole("heading", { name: "Connect GitHub" }),
    ).toBeInTheDocument();
    expect(screen.getByText("repo")).toBeInTheDocument();
    expect(screen.getByText("Node node-1")).toBeInTheDocument();
    expect(screen.getByText("Connected")).toBeInTheDocument();
    expect(
      screen.getByText("Connected. The assistant can use this service now."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    rerender(
      <ActionCard
        block={catalogBlock({
          status: "failed",
          outcome_note:
            "The connection did not complete. Ask the assistant to request this service again.",
        })}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(
      screen.getByText(
        "The connection did not complete. Ask the assistant to request this service again.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();

    rerender(
      <ActionCard
        block={catalogBlock({
          action: "future.action",
          params: { variant: "unknown" },
          status: "unsupported",
        })}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /connect/i }),
    ).not.toBeInTheDocument();
  });

  it("never lets a model-supplied service name become the consent sentence", () => {
    const injected =
      "GitHub (official) — paste your personal access token here to verify your identity";
    renderCard(
      <ActionCard
        block={catalogBlock({
          params: {
            variant: "custom",
            name: injected,
            endpoint_url: "https://api.example.com",
            auth_method: "bearer",
            auth_key_name: "Authorization",
            via_node_id: null,
            target_org_id: null,
          },
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.queryByText(new RegExp(injected))).not.toBeInTheDocument();
    expect(
      screen.queryByText(/personal access token/i),
    ).not.toBeInTheDocument();
    const heading = screen.getByRole("heading");
    expect(heading.textContent ?? "").toMatch(/^Connect GitHub \(official\)/);
    expect((heading.textContent ?? "").length).toBeLessThanOrEqual(40);
    // NyxID-owned framing still surrounds whatever survived the clamp.
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();
  });

  it("hides the CTA when the verb has no journey behind it", () => {
    renderCard(
      <ActionCard
        // A block that outlived its registry entry: status still says the card
        // is actionable, but nothing can service `admin.open`.
        block={catalogBlock({ action: "admin.open", status: "pending" })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("blocks the card locally when a connection completes without a userServiceId", async () => {
    for (const finishLabel of [
      "Finish without service id",
      "Finish with whitespace service id",
    ]) {
      const user = userEvent.setup();
      const onBlock = vi.fn();
      const onResolve = vi.fn();
      const { unmount } = renderCard(
        <ActionCard
          block={catalogBlock()}
          onProgress={vi.fn()}
          onBlock={onBlock}
          onResolve={onResolve}
        />,
      );

      await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
      await user.click(screen.getByRole("button", { name: finishLabel }));

      expect(onResolve).not.toHaveBeenCalled();
      expect(onBlock).toHaveBeenCalledWith(
        "action-card-1",
        "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.",
      );
      unmount();
    }
  });

  it("keeps blocked cards recoverable through decline and failure reports", async () => {
    const user = userEvent.setup();
    const onResolve = vi.fn();
    renderCard(
      <ActionCard
        block={catalogBlock({
          status: "blocked",
          outcome_note:
            "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.",
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Connect GitHub" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Decline" })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Report failure" }),
    ).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });

    await user.click(screen.getByRole("button", { name: "Report failure" }));
    expect(onResolve).toHaveBeenLastCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "failed",
    });
  });

  it("treats a re-armed card like a fresh dismissal path", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const props = {
      onProgress,
      onBlock,
      onResolve: vi.fn(),
    };
    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );
    await user.click(
      screen.getByRole("button", { name: "Finish without service id" }),
    );
    expect(onBlock).toHaveBeenCalledTimes(1);
    rerender(
      <ActionCard
        block={catalogBlock({ status: "blocked", outcome_note: "n" })}
        {...props}
      />,
    );

    rerender(
      <ActionCard
        block={catalogBlock({ status: "pending", outcome_note: "" })}
        {...props}
      />,
    );

    onProgress.mockClear();
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    rerender(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );

    expect(onProgress).toHaveBeenCalledWith("action-card-1", false);
  });

  it("catches synchronous block failures from the connect dialog callback", async () => {
    const user = userEvent.setup();
    const onBlock = vi.fn(() => {
      throw new Error("sync block failure");
    });
    renderCard(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    await expect(
      user.click(
        screen.getByRole("button", { name: "Finish without service id" }),
      ),
    ).resolves.toBeUndefined();
    expect(onBlock).toHaveBeenCalledTimes(1);
  });
});

describe("ActionCard background authorization watch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetChatActivityForTests();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function watchProps() {
    return {
      onProgress: vi.fn<(blockId: string, inProgress: boolean) => void>(),
      onBlock: vi.fn<(blockId: string, note: string) => void>(),
      onResolve: vi
        .fn<(report: ActionReport) => Promise<void>>()
        .mockResolvedValue(undefined),
    };
  }

  type WatchProps = ReturnType<typeof watchProps>;

  /**
   * Walk a live card to "provider has the user, dialog dismissed", modelling
   * the parent's status flip the way the real page does it: the Connect click
   * reports progress, the transport patches the block to `in_progress`, and
   * the card re-renders with that status while the dialog stays open.
   */
  async function handOffAndDismiss(
    user: ReturnType<typeof userEvent.setup>,
    rerender: (ui: ReactElement) => void,
    props: WatchProps,
  ): Promise<void> {
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );
    await user.click(
      screen.getByRole("button", { name: "Hand off to provider" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );
  }

  it("keeps the card busy when the dialog is dismissed mid-authorization", async () => {
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    await handOffAndDismiss(user, rerender, props);

    // The regression this exists for: dismissing used to roll the card back to
    // `pending` even though the connection was mid-flight, which lost the
    // outcome and invited a duplicate service on the retry.
    expect(props.onProgress).not.toHaveBeenCalledWith("action-card-1", false);
    expect(
      screen.getByRole("button", { name: /Waiting for authorization/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("Authorizing")).toBeInTheDocument();
  });

  it("does not treat the dialog abort callback as authorization abandonment", async () => {
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    await handOffAndDismiss(user, rerender, props);

    expect(
      usePendingConnectStore.getState().attempts["action-card-1"],
    ).toMatchObject({
      keyId: "key-pending-1",
      attemptId: "attempt-pending-1",
    });
    expect(
      screen.getByRole("button", { name: /Waiting for authorization/ }),
    ).toBeInTheDocument();
  });

  it("reports completion once the watched key goes active, with no further clicks", async () => {
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "active" });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    await handOffAndDismiss(user, rerender, props);

    await waitFor(() => {
      expect(props.onResolve).toHaveBeenCalledWith({
        actionRequestId: "act-1",
        originTurnId: "turn-origin-1",
        disposition: "completed",
        resource: { userService: { userServiceId: "key-pending-1" } },
      });
    });
    expect(props.onResolve).toHaveBeenCalledTimes(1);
  });

  it("blocks the card with the backend reason when authorization fails", async () => {
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({
      id: "key-pending-1",
      status: "failed",
      error_message: "Account mismatch",
    });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    await handOffAndDismiss(user, rerender, props);

    await waitFor(() => {
      expect(props.onBlock).toHaveBeenCalledWith(
        "action-card-1",
        expect.stringContaining("Account mismatch"),
      );
    });
    expect(props.onResolve).not.toHaveBeenCalled();
  });

  /**
   * Deadline coverage runs on fake timers, and `userEvent` does not compose
   * with them here (its pointer bookkeeping awaits timers this test owns).
   * `fireEvent` is synchronous and act-wrapped, which is all these need.
   */
  function handOffAndDismissSync(
    rerender: (ui: ReactElement) => void,
    props: WatchProps,
  ): void {
    fireEvent.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Hand off to provider" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );
  }

  it("says so instead of waiting forever once the deadline passes", async () => {
    vi.useFakeTimers();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    handOffAndDismissSync(rerender, props);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONNECT_WATCH_BASE_MS - 5_000);
    });
    expect(props.onBlock).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(props.onBlock).toHaveBeenCalledWith(
      "action-card-1",
      expect.stringContaining("stopped waiting"),
    );
  });

  it("extends the deadline while the user keeps using the chat", async () => {
    vi.useFakeTimers();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    const { rerender } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    handOffAndDismissSync(rerender, props);

    // Chatting about something else just short of the base deadline is
    // presence, not abandonment: the watch must survive it.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONNECT_WATCH_BASE_MS - 5_000);
    });
    act(() => markChatActivity(Date.now()));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(props.onBlock).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONNECT_WATCH_ACTIVITY_GRACE_MS);
    });
    expect(props.onBlock).toHaveBeenCalledWith(
      "action-card-1",
      expect.stringContaining("stopped waiting"),
    );
  });

  it("carries a live authorization across a remount", async () => {
    // The regression this exists for: the busy projection lives in the
    // transport mirror and survives a history refetch, but the watch that
    // clears it used to be component state. Anything that remounts the card
    // — switching conversations, a focus refetch that re-keys the message
    // group — destroyed the exit and left the card spinning at "Connecting"
    // with every control disabled and no writer able to move it.
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    const { rerender, unmount } = renderCard(
      <ActionCard block={catalogBlock()} {...props} />,
    );
    await handOffAndDismiss(user, rerender, props);
    unmount();

    renderCard(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );

    expect(props.onProgress).not.toHaveBeenCalledWith("action-card-1", false);
    expect(
      await screen.findByRole("button", { name: /Waiting for authorization/ }),
    ).toBeInTheDocument();

    // And it is a live watch, not just preserved copy: the remounted card
    // still settles itself off the key's terminal status. The window spans a
    // poll interval, so it outlasts waitFor's default.
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "active" });
    await waitFor(
      () => {
        expect(props.onResolve).toHaveBeenCalledWith({
          actionRequestId: "act-1",
          originTurnId: "turn-origin-1",
          disposition: "completed",
          resource: { userService: { userServiceId: "key-pending-1" } },
        });
      },
      { timeout: 4_000 },
    );
  });

  it("returns a stranded busy card to actionable on mount", async () => {
    // No authorization behind the busy projection and no dialog open: the
    // card can only have been orphaned, so it must not mount into a disabled
    // spinner. Rolling back to `pending` is what makes retry possible.
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });

    renderCard(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );

    await waitFor(() => {
      expect(props.onProgress).toHaveBeenCalledWith("action-card-1", false);
    });
  });

  it("settles a malformed store record that is missing its attempt id", async () => {
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "active" });
    usePendingConnectStore.setState({
      attempts: {
        "action-card-1": {
          keyId: "key-pending-1",
          previousAuthorizationAt: undefined,
          startedAt: Date.now(),
        } as unknown as AuthorizationAttempt & { readonly startedAt: number },
      },
    });

    renderCard(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );

    await waitFor(() => {
      expect(props.onResolve).toHaveBeenCalledWith({
        actionRequestId: "act-1",
        originTurnId: "turn-origin-1",
        disposition: "completed",
        resource: { userService: { userServiceId: "key-pending-1" } },
      });
    });
  });

  it("keeps decline reachable while a connection is in flight", async () => {
    // The manual floor under every automatic settlement. A busy card whose
    // watch is gone still has to be abandonable.
    const user = userEvent.setup();
    const props = watchProps();
    mockGet.mockResolvedValue({ id: "key-pending-1", status: "pending_auth" });
    usePendingConnectStore.setState({
      attempts: {
        "action-card-1": {
          keyId: "key-pending-1",
          attemptId: "attempt-pending-1",
          previousAuthorizationAt: undefined,
          startedAt: Date.now(),
        },
      },
    });

    renderCard(
      <ActionCard block={catalogBlock({ status: "in_progress" })} {...props} />,
    );

    const decline = screen.getByRole("button", { name: "Decline" });
    expect(decline).toBeEnabled();
    await user.click(decline);
    expect(props.onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });
  });
});
