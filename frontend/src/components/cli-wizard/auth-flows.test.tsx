import { render, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
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
      .mockResolvedValueOnce({ status: "failed" });
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
    expect(onTerminalFailure).toHaveBeenCalledWith("failed");
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
