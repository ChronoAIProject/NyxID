import { render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  isTerminalAuthFailureStatus,
  pollOAuthKeyUntilActive,
} from "./auth-flow-polling";
import { DeviceCodeFlow, OAuthFlow } from "./auth-flows";

const {
  mockDelete,
  mockGet,
  mockPost,
  mockReservePairingAction,
  mockRewindPairingAction,
} = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockReservePairingAction: vi.fn(),
  mockRewindPairingAction: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
    post: mockPost,
  },
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

vi.mock("@/pages/cli-pair/reserve-action", () => ({
  reservePairingAction: mockReservePairingAction,
  rewindPairingAction: mockRewindPairingAction,
}));

function resetFlowMocks() {
  mockDelete.mockReset();
  mockGet.mockReset();
  mockPost.mockReset();
  mockReservePairingAction.mockReset();
  mockRewindPairingAction.mockReset();
  mockReservePairingAction.mockResolvedValue(undefined);
  mockRewindPairingAction.mockResolvedValue(undefined);
  mockPost.mockImplementation(async (path: string) => {
    if (path === "/keys") {
      return {
        id: "key-1",
        status: "active",
        slug: "llm-openai",
        label: "OpenAI",
      };
    }
    throw new Error(`unexpected POST ${path}`);
  });
  mockGet.mockImplementation(async (path: string) => {
    if (path === "/keys/key-1") {
      return {
        id: "key-1",
        status: "active",
        slug: "llm-openai",
        label: "OpenAI",
      };
    }
    throw new Error(`unexpected GET ${path}`);
  });
}

function renderOAuthFlow(props: Partial<ComponentProps<typeof OAuthFlow>> = {}) {
  return render(
    <OAuthFlow
      providerId="provider-1"
      slug="llm-openai"
      label="OpenAI"
      pairingId="pair-1"
      onSuccess={vi.fn()}
      onCancel={vi.fn()}
      {...props}
    />,
  );
}

function renderDeviceCodeFlow(
  props: Partial<ComponentProps<typeof DeviceCodeFlow>> = {},
) {
  return render(
    <DeviceCodeFlow
      providerId="provider-1"
      slug="llm-openai"
      label="OpenAI"
      pairingId="pair-1"
      onSuccess={vi.fn()}
      onCancel={vi.fn()}
      {...props}
    />,
  );
}

function keyCreateBody(): Record<string, unknown> {
  const call = mockPost.mock.calls.find(([path]) => path === "/keys");
  if (!call) throw new Error("missing POST /keys call");
  return call[1] as Record<string, unknown>;
}

describe("cli wizard auth flows", () => {
  it.each(["revoked", "failed", "expired"] as const)(
    "treats %s as a terminal auth failure status",
    (status) => {
      expect(isTerminalAuthFailureStatus(status)).toBe(true);
    },
  );

  it("does not treat active or pending_auth as terminal auth failures", () => {
    expect(isTerminalAuthFailureStatus("active")).toBe(false);
    expect(isTerminalAuthFailureStatus("pending_auth")).toBe(false);
    expect(isTerminalAuthFailureStatus(undefined)).toBe(false);
  });

  it("stops OAuth polling when provider denial marks the placeholder failed", async () => {
    const getKey = vi
      .fn()
      .mockResolvedValueOnce({ status: "pending_auth" })
      .mockResolvedValueOnce({
        status: "failed",
        error_message: "Session mismatch",
      });
    const completeWithKey = vi.fn();
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();
    const sleepMs = vi.fn().mockResolvedValue(undefined);

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs,
    });

    expect(getKey).toHaveBeenCalledTimes(2);
    expect(completeWithKey).not.toHaveBeenCalled();
    expect(onTerminalFailure).toHaveBeenCalledWith({
      status: "failed",
      error_message: "Session mismatch",
    });
    expect(onTimeout).not.toHaveBeenCalled();
  });

  it("completes OAuth polling when the placeholder becomes active", async () => {
    const getKey = vi.fn().mockResolvedValue({ status: "active" });
    const completeWithKey = vi.fn().mockResolvedValue(undefined);
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
    });

    expect(completeWithKey).toHaveBeenCalledWith("key-1");
    expect(onTerminalFailure).not.toHaveBeenCalled();
    expect(onTimeout).not.toHaveBeenCalled();
  });

  // Manage-scopes mode (NyxID#917): the connection is already `active`, so
  // completion is a CHANGE in `last_authorized_at`, not `status === active`.
  // This is the Codex-review fix — the earlier granted-scopes-diff signal hung
  // forever when a real re-auth left the scopes unchanged. The predicate below
  // mirrors OAuthFlow's reconnect isComplete.
  const reconnectIsComplete =
    (baseline: string | null) =>
    (k: { status: string; last_authorized_at?: string | null }) =>
      k.status === "active" &&
      !!k.last_authorized_at &&
      k.last_authorized_at !== baseline;

  it("waits for last_authorized_at to advance, not status==active", async () => {
    const baseline = "2026-06-16T00:00:00Z";
    const getKey = vi
      .fn()
      // already active with the SAME timestamp → user hasn't re-consented yet
      .mockResolvedValueOnce({ status: "active", last_authorized_at: baseline })
      // fresh authorization stamped a new timestamp → complete
      .mockResolvedValueOnce({ status: "active", last_authorized_at: "2026-06-16T00:05:00Z" });
    const completeWithKey = vi.fn().mockResolvedValue(undefined);
    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure: vi.fn(),
      onTimeout: vi.fn(),
      sleepMs: vi.fn().mockResolvedValue(undefined),
      isComplete: reconnectIsComplete(baseline),
    });
    expect(getKey).toHaveBeenCalledTimes(2);
    expect(completeWithKey).toHaveBeenCalledWith("key-1");
  });

  it("completes a re-auth even when granted scopes are UNCHANGED (Codex P1 regression)", async () => {
    const baseline = "2026-06-16T00:00:00Z";
    // Scopes identical before and after; only last_authorized_at moves. The
    // old scope-diff signal would never complete here.
    const getKey = vi.fn().mockResolvedValue({
      status: "active",
      granted_scopes: ["tweet.read", "tweet.write"],
      last_authorized_at: "2026-06-16T00:09:00Z",
    });
    const completeWithKey = vi.fn().mockResolvedValue(undefined);
    const onTimeout = vi.fn();
    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure: vi.fn(),
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
      isComplete: reconnectIsComplete(baseline),
    });
    expect(completeWithKey).toHaveBeenCalledWith("key-1");
    expect(onTimeout).not.toHaveBeenCalled();
  });

  // Issue #653 stale-tab path: a 404 means the placeholder no longer
  // exists (abandoned by another tab, hard-deleted, never persisted).
  // Treat as terminal so the wizard exits with a clear message instead
  // of polling silently for the full 5-minute deadline.
  it("treats a 404 from polling as a terminal failure", async () => {
    const getKey = vi
      .fn()
      .mockRejectedValue(
        new ApiError(404, {
          error: "not_found",
          error_code: 1004,
          message: "Key not found",
        }),
      );
    const completeWithKey = vi.fn();
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
    });

    expect(getKey).toHaveBeenCalledTimes(1);
    expect(completeWithKey).not.toHaveBeenCalled();
    expect(onTimeout).not.toHaveBeenCalled();
    expect(onTerminalFailure).toHaveBeenCalledTimes(1);
    const call = onTerminalFailure.mock.calls[0]?.[0] as {
      status: string;
      error_message?: string | null;
    };
    expect(call.status).toBe("failed");
    expect(call.error_message).toMatch(/no longer exists/i);
  });

  // Non-404 fetch errors (transient network blips, 5xx, refresh-token
  // churn) must remain transient — keep polling, not exit.
  it("keeps polling on transient (non-404) fetch errors", async () => {
    const getKey = vi
      .fn()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({ status: "active" });
    const completeWithKey = vi.fn().mockResolvedValue(undefined);
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
    });

    expect(getKey).toHaveBeenCalledTimes(2);
    expect(completeWithKey).toHaveBeenCalledWith("key-1");
    expect(onTerminalFailure).not.toHaveBeenCalled();
    expect(onTimeout).not.toHaveBeenCalled();
  });

  // Issue #653 — the wizard MUST reach a terminal state for every
  // outcome. After enough consecutive non-success polls (e.g. the
  // wizard's local CLI server died, or backend is sustained-down), give
  // up and surface a "lost contact" message so the user sees something
  // actionable instead of "Waiting…" forever.
  it("escalates to terminal failure after sustained polling errors", async () => {
    const getKey = vi
      .fn()
      .mockRejectedValue(new Error("network down"));
    const completeWithKey = vi.fn();
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
      maxConsecutiveErrors: 3,
    });

    expect(getKey).toHaveBeenCalledTimes(3);
    expect(completeWithKey).not.toHaveBeenCalled();
    expect(onTimeout).not.toHaveBeenCalled();
    expect(onTerminalFailure).toHaveBeenCalledTimes(1);
    const call = onTerminalFailure.mock.calls[0]?.[0] as {
      status: string;
      error_message?: string | null;
    };
    expect(call.status).toBe("failed");
    expect(call.error_message).toMatch(/lost contact|nyxid status/i);
  });

  // The consecutive-error counter must RESET on a successful poll —
  // intermittent blips during a long OAuth flow shouldn't trip the
  // escalation if interspersed with healthy responses.
  it("resets the consecutive-error counter when a poll succeeds", async () => {
    const getKey = vi
      .fn()
      .mockRejectedValueOnce(new Error("blip 1"))
      .mockRejectedValueOnce(new Error("blip 2"))
      .mockResolvedValueOnce({ status: "pending_auth" })
      .mockRejectedValueOnce(new Error("blip 3"))
      .mockResolvedValueOnce({ status: "active" });
    const completeWithKey = vi.fn().mockResolvedValue(undefined);
    const onTerminalFailure = vi.fn();
    const onTimeout = vi.fn();

    await pollOAuthKeyUntilActive({
      keyId: "key-1",
      getKey,
      completeWithKey,
      isCancelled: () => false,
      onTerminalFailure,
      onTimeout,
      sleepMs: vi.fn().mockResolvedValue(undefined),
      maxConsecutiveErrors: 3,
    });

    expect(getKey).toHaveBeenCalledTimes(5);
    expect(completeWithKey).toHaveBeenCalledWith("key-1");
    expect(onTerminalFailure).not.toHaveBeenCalled();
    expect(onTimeout).not.toHaveBeenCalled();
  });

  it("posts target_org_id when creating an OAuth placeholder under an org", async () => {
    resetFlowMocks();

    renderOAuthFlow({
      targetOrgId: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
    });

    await waitFor(() => {
      expect(keyCreateBody()).toMatchObject({
        service_slug: "llm-openai",
        label: "OpenAI",
        target_org_id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
      });
    });
  });

  it("posts target_org_id when creating a device-code placeholder under an org", async () => {
    resetFlowMocks();

    renderDeviceCodeFlow({
      targetOrgId: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
    });

    await waitFor(() => {
      expect(keyCreateBody()).toMatchObject({
        service_slug: "llm-openai",
        label: "OpenAI",
        target_org_id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
      });
    });
  });

  it.each([undefined, null] as const)(
    "omits target_org_id when OAuth targetOrgId is %s",
    async (targetOrgId) => {
      resetFlowMocks();

      renderOAuthFlow({ targetOrgId });

      await waitFor(() => {
        expect(keyCreateBody()).not.toHaveProperty("target_org_id");
      });
    },
  );
});

// Issue #653 — root-cause regression test for the polling-doesn't-fire
// bug. PR #723's third-round review caught that OAuthFlow's main
// useEffect had `[phase]` deps and its cleanup set `cancelledRef.
// current = true`. When the async function inside calls
// `setPhase("waiting")` then `await pollUntilActive(...)`, React fires
// the cleanup during the polling loop's first sleep — which flips
// cancelledRef and makes the polling loop bail before its first GET.
// Result: zero `/keys/<id>` requests in the network tab, wizard hangs
// on "Waiting for provider authorization…" indefinitely.
//
// Pre-fix this test fails (mockGet for /keys/key-1 is never called).
// Post-fix it passes (cleanup no longer flips the shared
// cancelledRef on phase change).
//
// Existing OAuth integration tests didn't catch this because their
// `mockPost` returned `status: "active"` immediately, hitting the
// short-circuit at auth-flows.tsx:857 that bypasses pollUntilActive
// entirely. This test deliberately starts with `pending_auth` so the
// polling loop is the path that has to work.
describe("OAuthFlow polling integration", () => {
  beforeEach(() => {
    resetFlowMocks();
  });

  // Regression guard: the UI consistency sweep (5f9a67e / PR #733)
  // reverted the cancelledRef-in-cleanup fix from PR #723 (issue #653
  // root cause). The fix is restored on this branch — the [phase]-deps
  // useEffect cleanup must NOT flip `cancelledRef.current` (only the
  // unmount-only effect does), otherwise setPhase("waiting") aborts
  // polling before the first GET fires.
  it(
    "actually fires GET /keys/<id> while the placeholder is pending_auth (issue #653 root cause)",
    async () => {
      // Override defaults: POST /keys returns pending_auth so the
      // active short-circuit doesn't fire; GET /keys/key-1 also
      // returns pending_auth so the polling has to keep going for
      // multiple ticks (we only assert the first GET fires; the
      // wizard's done transition isn't what's being tested here).
      mockPost.mockImplementation(async (path: string) => {
        if (path === "/keys") {
          return {
            id: "key-1",
            status: "pending_auth",
            slug: "llm-openai",
            label: "OpenAI",
          };
        }
        throw new Error(`unexpected POST ${path}`);
      });
      mockGet.mockImplementation(async (path: string) => {
        // The wizard now also threads the placeholder's id into the
        // initiate URL as `&key_id=<id>` (multi-connection routing —
        // tells the backend callback which UserApiKey to write the
        // tokens onto). Match either shape so the test doesn't have
        // to mirror that detail.
        if (
          path.startsWith("/providers/") &&
          path.includes("/oauth?redirect_path=%2Fkeys%2Fkey-1")
        ) {
          return { authorization_url: "https://example.com/oauth" };
        }
        if (path === "/keys/key-1") {
          return {
            id: "key-1",
            status: "pending_auth",
            slug: "llm-openai",
            label: "OpenAI",
          };
        }
        throw new Error(`unexpected GET ${path}`);
      });

      // Note: no need to stub `window.open` — issue #653 Option A
      // removed the auto-window.open from the wizard's effect. The
      // OAuth URL is rendered as the prominent "Open {provider} sign-
      // in" button instead, and polling fires regardless of whether
      // that button has been clicked.
      renderOAuthFlow();

      // Wait for the polling loop to fire its first GET. Default
      // polling interval is 2s; allow generous slack for CI.
      await waitFor(
        () => {
          expect(mockGet).toHaveBeenCalledWith("/keys/key-1");
        },
        { timeout: 5000 },
      );
    },
    10_000,
  );
});

// Issue #653 review (PR #723 second-round adversarial review): when
// the OAuth or device-code flow has reached a terminal error, the
// wizard MUST render a dedicated error layout — not the polling
// waiting panel with its spinner and "Open provider sign-in" button
// still active. Showing the spinner would lie about the flow still
// being in progress; showing the open button would invite the user
// to retry an authorization URL the backend has already abandoned.
describe("OAuthFlow error phase", () => {
  beforeEach(() => {
    resetFlowMocks();
  });

  it("renders the error layout (no spinner, no open button) when phase is error", async () => {
    // Force the flow into the error phase by making placeholder
    // creation fail — the catch block sets phase = "error" and
    // surfaces the message.
    mockPost.mockReset();
    mockPost.mockRejectedValueOnce(
      new Error("backend rejected the placeholder create"),
    );
    renderOAuthFlow();
    await waitFor(() => {
      expect(
        screen.getByText(/backend rejected the placeholder create/i),
      ).toBeTruthy();
    });

    // Polling spinner copy (from the waiting panel) must NOT be shown.
    expect(
      screen.queryByText(/Waiting for provider authorization/i),
    ).toBeNull();
    expect(
      screen.queryByText(/Setting up placeholder service/i),
    ).toBeNull();
    expect(screen.queryByText(/Checking provider credentials/i)).toBeNull();

    // The "Open … sign-in" button must NOT be rendered — there's
    // nothing useful to open at this point.
    expect(
      screen.queryByRole("button", { name: /Open .* sign-in/i }),
    ).toBeNull();

    // Cancel button stays available so the user can exit cleanly.
    expect(screen.getByRole("button", { name: /^Cancel$/ })).toBeTruthy();
  });
});

// NyxID#917 — the pair wizard forwards the scope-picker selection as the
// `scope_override` query param (the COMPLETE set, replacing the provider's
// default scopes) on the OAuth / device-code initiate request. `undefined`
// means the provider doesn't accept scopes, so no scope param is sent.
describe("upstream scope override forwarding (issue #917)", () => {
  beforeEach(() => {
    resetFlowMocks();
  });

  /** Wire mocks so the flow reaches the initiate request instead of
   *  short-circuiting on an already-active placeholder. */
  function arrangePendingPlaceholder() {
    mockPost.mockImplementation(async (path: string) => {
      if (path === "/keys") {
        return {
          id: "key-1",
          status: "pending_auth",
          slug: "llm-openai",
          label: "OpenAI",
        };
      }
      if (path.includes("/connect/device-code/initiate")) {
        return {
          user_code: "ABCD-1234",
          verification_uri: "https://example.com/device",
          state: "state-1",
          interval: 5,
          expires_in: 900,
        };
      }
      throw new Error(`unexpected POST ${path}`);
    });
    mockGet.mockImplementation(async (path: string) => {
      if (path.startsWith("/providers/") && path.includes("/connect/oauth?")) {
        return { authorization_url: "https://example.com/oauth" };
      }
      if (path === "/keys/key-1") {
        return {
          id: "key-1",
          status: "pending_auth",
          slug: "llm-openai",
          label: "OpenAI",
        };
      }
      throw new Error(`unexpected GET ${path}`);
    });
  }

  it("sends scope_override (complete set) on the OAuth initiate URL", async () => {
    arrangePendingPlaceholder();

    renderOAuthFlow({
      scopeOverride: ["media.write", "tweet.read"],
    });

    await waitFor(() => {
      const initiateCall = mockGet.mock.calls.find(([path]) =>
        (path as string).includes("/connect/oauth?"),
      );
      expect(initiateCall).toBeTruthy();
      const url = initiateCall?.[0] as string;
      const query = new URLSearchParams(url.split("?")[1]);
      expect(query.get("scope_override")).toBe("media.write,tweet.read");
      // Additive `scope` is not used by the picker path.
      expect(query.get("scope")).toBeNull();
      // Existing params survive the rewrite to URLSearchParams.
      expect(query.get("key_id")).toBe("key-1");
      expect(query.get("redirect_path")).toBe("/keys/key-1");
    });
  });

  it("sends an empty scope_override when the user cleared every scope", async () => {
    arrangePendingPlaceholder();

    renderOAuthFlow({ scopeOverride: [] });

    await waitFor(() => {
      const initiateCall = mockGet.mock.calls.find(([path]) =>
        (path as string).includes("/connect/oauth?"),
      );
      expect(initiateCall).toBeTruthy();
      const url = initiateCall?.[0] as string;
      const query = new URLSearchParams(url.split("?")[1]);
      // Present-but-empty: the backend reads this as "drop all scopes".
      expect(query.get("scope_override")).toBe("");
    });
  });

  it("omits scope params from the OAuth initiate URL when scopeOverride is undefined", async () => {
    arrangePendingPlaceholder();

    renderOAuthFlow();

    await waitFor(() => {
      const initiateCall = mockGet.mock.calls.find(([path]) =>
        (path as string).includes("/connect/oauth?"),
      );
      expect(initiateCall).toBeTruthy();
      const url = initiateCall?.[0] as string;
      const query = new URLSearchParams(url.split("?")[1]);
      expect(query.get("scope_override")).toBeNull();
      expect(query.get("scope")).toBeNull();
    });
  });

  it("sends scope_override on the device-code initiate URL", async () => {
    arrangePendingPlaceholder();

    renderDeviceCodeFlow({
      scopeOverride: ["repo", "read:org"],
    });

    await waitFor(() => {
      const initiateCall = mockPost.mock.calls.find(([path]) =>
        (path as string).includes("/connect/device-code/initiate"),
      );
      expect(initiateCall).toBeTruthy();
      const url = initiateCall?.[0] as string;
      const query = new URLSearchParams(url.split("?")[1]);
      expect(query.get("scope_override")).toBe("repo,read:org");
      expect(query.get("key_id")).toBe("key-1");
    });
  });

  it("omits scope params from the device-code initiate URL when scopeOverride is undefined", async () => {
    arrangePendingPlaceholder();

    renderDeviceCodeFlow();

    await waitFor(() => {
      const initiateCall = mockPost.mock.calls.find(([path]) =>
        (path as string).includes("/connect/device-code/initiate"),
      );
      expect(initiateCall).toBeTruthy();
      const url = initiateCall?.[0] as string;
      const query = new URLSearchParams(url.split("?")[1]);
      expect(query.get("scope_override")).toBeNull();
      expect(query.get("scope")).toBeNull();
    });
  });
});
