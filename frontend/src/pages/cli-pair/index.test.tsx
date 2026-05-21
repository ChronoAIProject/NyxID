import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClaimResponse, PairingKind } from "./types";

// Orchestration tests for the Mode-B `/cli/pair` wizard page. The page
// itself owns the state machine (enter-code → claimed → notifying/secret/
// acking → done, plus the resumed-* recovery branches) and the
// claim/complete/cancel/poll API calls. Heavy per-kind child panels are
// stubbed to lightweight props-exposing harnesses so we can drive the
// orchestration without their internals: each ConfirmPanel stub exposes a
// button that fires `onSuccess(result)`, and the DisplayOnce stub exposes
// an "ack" button that fires the parent's acknowledge callback.

const { api, ApiError, storeState, stepState } = vi.hoisted(() => {
  class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(status: number, message: string, errorCode = 0) {
      super(message);
      this.status = status;
      this.errorCode = errorCode;
    }
  }
  return {
    api: { post: vi.fn(), get: vi.fn() },
    ApiError,
    storeState: { isAuthenticated: true, isLoading: false },
    // Captured from the stubbed WizardShell so tests can assert which
    // step the resolver picked for the current (flow, phase).
    stepState: { current: null as null | { current: number; total: number; label: string } },
  };
});

vi.mock("@/lib/api-client", () => ({ api, ApiError }));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => storeState,
}));

// Shell: pass children through and surface the resolved step so tests
// can pin "which step renders for which flow".
vi.mock("@/components/cli-wizard/shell", () => ({
  WizardShell: ({
    children,
    step,
  }: {
    children: React.ReactNode;
    step?: { current: number; total: number; label: string };
  }) => {
    stepState.current = step ?? null;
    return (
      <div data-testid="wizard-shell" data-step={step ? JSON.stringify(step) : ""}>
        {children}
      </div>
    );
  },
}));

// Disconnect banner: surface the pairingStatus so the disconnect-banner
// assertions don't depend on the real banner's copy.
vi.mock("@/components/cli-wizard/disconnect-banner", () => ({
  DisconnectBanner: ({
    pairingStatus,
  }: {
    pairingStatus?: "cancelled" | "expired" | "unknown";
  }) => (
    <div data-testid="disconnect-banner" data-status={pairingStatus ?? ""}>
      disconnected
    </div>
  ),
}));

// Per-kind confirm panels (Step 2). Each stub renders a button that, when
// clicked, fires `onSuccess` with a kind-appropriate ActionResult. The
// page's ConfirmPanel switch picks the right one based on `claim.kind`.
function confirmStub(testid: string, result: Record<string, unknown>) {
  return ({ onSuccess }: { onSuccess: (r: unknown) => void }) => (
    <button
      type="button"
      data-testid={testid}
      onClick={() => {
        onSuccess(result);
      }}
    >
      complete {testid}
    </button>
  );
}

vi.mock("@/components/cli-wizard/confirm-panels", () => ({
  ApiKeyCreateConfirm: confirmStub("confirm-api-key-create", {
    kind: "api-key-create",
    api_key_id: "ak-1",
    full_key: "nyxid_ak_secret",
  }),
  ApiKeyRotateConfirm: confirmStub("confirm-api-key-rotate", {
    kind: "api-key-rotate",
    resource_id: "ak-1",
    full_key: "nyxid_ak_rotated",
  }),
  NodeRegisterConfirm: confirmStub("confirm-node-register", {
    kind: "node-register-token",
    token_id: "tok-1",
    token: "nyx_nreg_secret",
  }),
  NodeRotateConfirm: confirmStub("confirm-node-rotate", {
    kind: "node-rotate-token",
    resource_id: "node-1",
    auth_token: "nyx_nauth_secret",
    signing_secret: "sign-secret",
  }),
  ServiceAccountCreateConfirm: confirmStub("confirm-sa-create", {
    kind: "service-account-create",
    service_account_id: "sa-1",
    client_id: "cid",
    client_secret: "csecret",
  }),
  ServiceAccountRotateSecretConfirm: confirmStub("confirm-sa-rotate", {
    kind: "service-account-rotate-secret",
    resource_id: "sa-1",
    client_id: "cid",
    client_secret: "csecret2",
  }),
  DeveloperAppCreateConfirm: confirmStub("confirm-dev-create", {
    kind: "developer-app-create",
    developer_app_id: "app-1",
    client_secret: "appsecret",
  }),
  DeveloperAppRotateSecretConfirm: confirmStub("confirm-dev-rotate", {
    kind: "developer-app-rotate-secret",
    resource_id: "app-1",
    client_secret: "appsecret2",
  }),
  MfaSetupConfirm: confirmStub("confirm-mfa", {
    kind: "mfa-setup",
    factor_id: "factor-1",
    recovery_codes: ["c1", "c2"],
  }),
}));

vi.mock("@/components/cli-wizard/ai-key-confirm-panel", () => ({
  AiKeyConfirm: ({
    onSuccess,
  }: {
    onSuccess: (r: unknown) => void;
  }) => (
    <button
      type="button"
      data-testid="confirm-ai-key"
      onClick={() => {
        onSuccess({
          kind: "ai-key",
          service_id: "svc-1",
          slug: "llm-openai",
          label: "OpenAI",
        });
      }}
    >
      complete ai-key
    </button>
  ),
}));

// DisplayOnce / RecoveryCodes (Step 3 secret render). Expose the rendered
// secret and an "ack" button that fires the parent's acknowledge callback
// so we can drive secret → done.
vi.mock("@/pages/cli-pair/display-once", () => ({
  DisplayOncePanel: ({
    title,
    secret,
    onAcknowledge,
  }: {
    title: string;
    secret: string;
    onAcknowledge: () => void;
  }) => (
    <div data-testid="display-once" data-title={title} data-secret={secret}>
      <button type="button" data-testid="ack-secret" onClick={onAcknowledge}>
        I saved it
      </button>
    </div>
  ),
  RecoveryCodesPanel: ({
    codes,
    onAcknowledged,
  }: {
    codes: readonly string[];
    onAcknowledged: () => void;
  }) => (
    <div data-testid="recovery-codes" data-codes={codes.join(",")}>
      <button type="button" data-testid="ack-recovery" onClick={onAcknowledged}>
        I saved them
      </button>
    </div>
  ),
}));

// parseAiKeyPrefill is only used to shape the prefill prop for the (stubbed)
// AiKeyConfirm; identity is enough here.
vi.mock("@/schemas/cli-wizard", () => ({
  parseAiKeyPrefill: (v: unknown) => v,
}));

import { CliPairPage } from "./index";

let assignSpy: ReturnType<typeof vi.spyOn>;

function makeClaim(
  kind: PairingKind,
  overrides: Partial<ClaimResponse> = {},
): ClaimResponse {
  return {
    id: "pair-1",
    kind,
    prefill: { resource_id: "res-1", display_name: "Thing" },
    resumed: false,
    action_started: false,
    ...overrides,
  };
}

/**
 * Drive the real EnterCodeForm: type a code, submit, and resolve the
 * `/cli-pairings/claim` POST with the given claim. Pending GET /poll calls
 * are answered with "claimed" so the liveness poller is inert.
 */
async function enterCode(
  user: ReturnType<typeof userEvent.setup>,
  claim: ClaimResponse,
  pollStatus: string = "claimed",
) {
  api.post.mockImplementation(async (path: string) => {
    if (path === "/cli-pairings/claim") return claim;
    throw new Error(`unexpected POST ${path}`);
  });
  // The liveness poller fires an immediate tick when the claim lands, so
  // a terminal pollStatus is observed without waiting for the 4s interval.
  api.get.mockResolvedValue({ status: pollStatus });

  await user.type(screen.getByLabelText(/Pairing code/i), "abcd-1234");
  await user.click(screen.getByRole("button", { name: /Continue/i }));
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useRealTimers();
  storeState.isAuthenticated = true;
  storeState.isLoading = false;
  stepState.current = null;
  window.history.pushState({}, "", "/cli/pair");
  assignSpy = vi
    .spyOn(window.location, "assign")
    .mockImplementation(() => {});
  api.get.mockResolvedValue({ status: "claimed" });
});

afterEach(() => {
  assignSpy.mockRestore();
  vi.useRealTimers();
});

describe("CliPairPage gating", () => {
  it("renders a loading skeleton (no enter-code form) while auth is loading", () => {
    storeState.isLoading = true;

    render(<CliPairPage />);

    expect(screen.queryByLabelText(/Pairing code/i)).not.toBeInTheDocument();
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it("redirects to /login preserving the full pair URL (path + ?code) when unauthenticated", async () => {
    storeState.isAuthenticated = false;
    storeState.isLoading = false;
    window.history.pushState({}, "", "/cli/pair?code=ABCD-1234");

    render(<CliPairPage />);

    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledTimes(1);
    });
    const target = assignSpy.mock.calls[0]![0] as string;
    const returnTo = decodeURIComponent(target.split("return_to=")[1]!);
    expect(returnTo).toBe(
      `${window.location.origin}/cli/pair?code=ABCD-1234`,
    );
  });
});

describe("CliPairPage enter-code step", () => {
  it("prefills the code from ?code and shows the confirm-from-URL copy", () => {
    window.history.pushState({}, "", "/cli/pair?code=abcd1234");

    render(<CliPairPage />);

    expect(screen.getByLabelText(/Pairing code/i)).toHaveValue("ABCD-1234");
    expect(
      screen.getByText(/We've filled in the code from the URL/i),
    ).toBeInTheDocument();
    // Pre-flow neutral step copy.
    expect(stepState.current).toEqual({
      current: 1,
      total: 3,
      label: "enter code",
    });
  });

  it("surfaces a claim error and cancels an unsupported-kind claim", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    const cancelCalls: string[] = [];
    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/claim") {
        return { id: "pair-x", kind: "future-kind", prefill: {}, resumed: false, action_started: false };
      }
      if (path === "/cli-pairings/pair-x/cancel") {
        cancelCalls.push(path);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.type(screen.getByLabelText(/Pairing code/i), "abcd-1234");
    await user.click(screen.getByRole("button", { name: /Continue/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/Unsupported pairing kind from server/i),
      ).toBeInTheDocument();
    });
    // The unsupported claim is cancelled so the CLI exits promptly.
    expect(cancelCalls).toEqual(["/cli-pairings/pair-x/cancel"]);
    // Still on the enter-code step — no flow chosen.
    expect(screen.getByLabelText(/Pairing code/i)).toBeInTheDocument();
  });

  it("shows the API error message when the claim POST rejects", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    api.post.mockRejectedValueOnce(new ApiError(404, "No such pairing code"));

    await user.type(screen.getByLabelText(/Pairing code/i), "abcd-1234");
    await user.click(screen.getByRole("button", { name: /Continue/i }));

    await waitFor(() => {
      expect(screen.getByText(/No such pairing code/i)).toBeInTheDocument();
    });
  });
});

describe("CliPairPage forward navigation per flow", () => {
  it("renders the api-key-create confirm panel for an api-key-create claim and steps to 'configure scope'", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("api-key-create"));

    await waitFor(() => {
      expect(screen.getByTestId("confirm-api-key-create")).toBeInTheDocument();
    });
    expect(stepState.current).toEqual({
      current: 2,
      total: 3,
      label: "configure scope",
    });
  });

  it("renders the ai-key confirm panel for an ai-key claim ('pick a service')", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("ai-key"));

    await waitFor(() => {
      expect(screen.getByTestId("confirm-ai-key")).toBeInTheDocument();
    });
    expect(stepState.current?.label).toBe("pick a service");
  });

  it("renders the node-register confirm panel for a node-register-token claim", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("node-register-token"));

    await waitFor(() => {
      expect(screen.getByTestId("confirm-node-register")).toBeInTheDocument();
    });
    expect(stepState.current?.label).toBe("name this node");
  });

  it("renders the mfa confirm panel for an mfa-setup claim", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("mfa-setup"));

    await waitFor(() => {
      expect(screen.getByTestId("confirm-mfa")).toBeInTheDocument();
    });
    expect(stepState.current?.label).toBe("scan and verify");
  });
});

describe("CliPairPage DisplayOnce complete wiring", () => {
  it("posts /complete with the api_key_id ack, shows the secret, then advances to done on ack", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("api-key-create"));

    // Reconfigure post: claim succeeds, /complete succeeds.
    const completeBodies: unknown[] = [];
    api.post.mockImplementation(async (path: string, body?: unknown) => {
      if (path === "/cli-pairings/pair-1/complete") {
        completeBodies.push(body);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(await screen.findByTestId("confirm-api-key-create"));

    // /complete fires immediately with the create ack.
    await waitFor(() => {
      expect(completeBodies).toEqual([
        { ack: { acknowledged: true, api_key_id: "ak-1" } },
      ]);
    });

    // After /complete resolves, the DisplayOnce secret renders.
    const secret = await screen.findByTestId("display-once");
    expect(secret).toHaveAttribute("data-title", "API key created");
    expect(secret).toHaveAttribute("data-secret", "nyxid_ak_secret");
    expect(stepState.current?.label).toBe("save the value");

    // Acknowledging advances to the done screen. The done stage carries
    // no claim, so the flow is unknown and the step resolver falls back
    // to the neutral pre-flow label (the DonePanel heading is the real
    // terminal signal here).
    await user.click(screen.getByTestId("ack-secret"));
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
    expect(stepState.current).toEqual({
      current: 1,
      total: 3,
      label: "enter code",
    });
  });

  it("renders recovery codes for an mfa-setup secret and advances to done", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("mfa-setup"));

    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/pair-1/complete") return {};
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(await screen.findByTestId("confirm-mfa"));

    const panel = await screen.findByTestId("recovery-codes");
    expect(panel).toHaveAttribute("data-codes", "c1,c2");

    await user.click(screen.getByTestId("ack-recovery"));
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });

  it("stays in notifying-cli with a retry that re-posts /complete when the first /complete fails", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("api-key-create"));

    let completeCalls = 0;
    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/pair-1/complete") {
        completeCalls += 1;
        if (completeCalls === 1) {
          throw new ApiError(503, "backend hiccup");
        }
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(await screen.findByTestId("confirm-api-key-create"));

    // First /complete fails → notifying-cli error + Retry button.
    await waitFor(() => {
      expect(screen.getByText(/Couldn't notify CLI: backend hiccup/i)).toBeInTheDocument();
    });
    expect(screen.queryByTestId("display-once")).not.toBeInTheDocument();

    // Retry re-posts /complete (now succeeds) → secret renders.
    await user.click(screen.getByRole("button", { name: /^Retry$/i }));
    expect(await screen.findByTestId("display-once")).toBeInTheDocument();
    expect(completeCalls).toBe(2);
  });
});

describe("CliPairPage ai-key acking wiring", () => {
  it("posts the ai-key ack and goes straight to done (no DisplayOnce)", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("ai-key"));

    const completeBodies: unknown[] = [];
    api.post.mockImplementation(async (path: string, body?: unknown) => {
      if (path === "/cli-pairings/pair-1/complete") {
        completeBodies.push(body);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(await screen.findByTestId("confirm-ai-key"));

    await waitFor(() => {
      expect(completeBodies).toEqual([
        {
          ack: {
            acknowledged: true,
            service_id: "svc-1",
            slug: "llm-openai",
            label: "OpenAI",
          },
        },
      ]);
    });
    // ai-key skips the DisplayOnce panel entirely.
    expect(screen.queryByTestId("display-once")).not.toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });

  it("surfaces the ai-key ack failure with a retry that re-posts /complete", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("ai-key"));

    let calls = 0;
    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/pair-1/complete") {
        calls += 1;
        if (calls === 1) throw new Error("net down");
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(await screen.findByTestId("confirm-ai-key"));

    await waitFor(() => {
      expect(screen.getByText(/Couldn't notify CLI: net down/i)).toBeInTheDocument();
    });
    await user.click(screen.getByRole("button", { name: /^Retry$/i }));
    await waitFor(() => {
      expect(calls).toBe(2);
    });
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });
});

describe("CliPairPage resumed recovery branches", () => {
  it("routes a resumed+action_started ROTATION claim to the rotation-choice panel and resends the ack on confirm", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(
      user,
      makeClaim("api-key-rotate", {
        resumed: true,
        action_started: true,
        prefill: { resource_id: "ak-99", display_name: "Key" },
      }),
    );

    // Recovery disambiguation panel, not the confirm panel.
    expect(
      await screen.findByRole("heading", { name: /already started/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Notify CLI/i })).toBeInTheDocument();
    expect(stepState.current?.label).toBe("Recovery · confirm outcome");

    const completeBodies: unknown[] = [];
    api.post.mockImplementation(async (path: string, body?: unknown) => {
      if (path === "/cli-pairings/pair-1/complete") {
        completeBodies.push(body);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(screen.getByRole("button", { name: /Notify CLI/i }));

    // Reconstructed rotation ack uses prefill.resource_id.
    await waitFor(() => {
      expect(completeBodies).toEqual([
        { ack: { acknowledged: true, resource_id: "ak-99" } },
      ]);
    });
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });

  it("cancels the pairing when the user reports the rotation did NOT succeed", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(
      user,
      makeClaim("node-rotate-token", {
        resumed: true,
        action_started: true,
        prefill: { resource_id: "node-7", display_name: "Node" },
      }),
    );

    await screen.findByRole("heading", { name: /already started/i });

    const cancelCalls: string[] = [];
    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/pair-1/cancel") {
        cancelCalls.push(path);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(screen.getByRole("button", { name: /Cancel Pairing/i }));

    await waitFor(() => {
      expect(cancelCalls).toEqual(["/cli-pairings/pair-1/cancel"]);
    });
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });

  it("routes a resumed+action_started CREATE claim to the warning panel (no ack reconstructable)", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(
      user,
      makeClaim("api-key-create", {
        resumed: true,
        action_started: true,
        prefill: {},
      }),
    );

    expect(
      await screen.findByRole("heading", { name: /already started/i }),
    ).toBeInTheDocument();
    // The create-warning panel offers a management link, not a "Notify CLI".
    expect(
      screen.getByRole("link", { name: /Open NyxID API Keys page/i }),
    ).toBeInTheDocument();
    expect(stepState.current?.label).toBe("Recovery · pairing already started");
    expect(screen.queryByTestId("confirm-api-key-create")).not.toBeInTheDocument();
  });

  it("cancels the pairing from the create-warning panel and advances to done", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(
      user,
      makeClaim("node-register-token", {
        resumed: true,
        action_started: true,
        prefill: {},
      }),
    );

    await screen.findByRole("heading", { name: /already started/i });

    const cancelCalls: string[] = [];
    api.post.mockImplementation(async (path: string) => {
      if (path === "/cli-pairings/pair-1/cancel") {
        cancelCalls.push(path);
        return {};
      }
      throw new Error(`unexpected POST ${path}`);
    });

    await user.click(screen.getByRole("button", { name: /Cancel Pairing/i }));

    await waitFor(() => {
      expect(cancelCalls).toEqual(["/cli-pairings/pair-1/cancel"]);
    });
    expect(
      await screen.findByRole("heading", { name: /Pairing complete/i }),
    ).toBeInTheDocument();
  });

  it("treats a resumed claim WITHOUT action_started as a normal confirm (recoverable refresh)", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(
      user,
      makeClaim("api-key-create", { resumed: true, action_started: false }),
    );

    // Pre-action resume → safe to re-render the confirm panel.
    expect(
      await screen.findByTestId("confirm-api-key-create"),
    ).toBeInTheDocument();
  });
});

describe("CliPairPage liveness poll → disconnect banner", () => {
  it("shows the cancelled disconnect banner when the poll reports the CLI cancelled", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    // The poll's immediate tick reports cancelled the moment the claim
    // lands, so the disconnect banner appears without the 4s interval.
    await enterCode(user, makeClaim("api-key-create"), "cancelled");

    const banner = await screen.findByTestId("disconnect-banner");
    expect(banner).toHaveAttribute("data-status", "cancelled");
  });

  it("shows the expired disconnect banner when the poll reports the pairing expired", async () => {
    const user = userEvent.setup();
    render(<CliPairPage />);

    await enterCode(user, makeClaim("api-key-create"), "expired");

    const banner = await screen.findByTestId("disconnect-banner");
    expect(banner).toHaveAttribute("data-status", "expired");
  });
});
