import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiKeyCreateResponse } from "@/types/api";
import { ConnectVerifyStep, type CreatedKey } from "./connect-verify-step";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";

/**
 * This suite is the regression fence for the "silent CTA" bug (Calvin,
 * 2026-06-30). The Footer used to return the handler reference from a
 * dispatch helper without invoking it, so clicking "Create Agent Key"
 * fired no mutation and no console log. These tests assert the
 * click→mutate→panel path end-to-end.
 */

const { createApiKeyMutate, toastFns } = vi.hoisted(() => ({
  createApiKeyMutate: vi.fn(),
  toastFns: { success: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/use-api-keys", () => ({
  useCreateApiKey: () => ({
    mutate: createApiKeyMutate,
    isPending: false,
  }),
}));

// CopyableField calls sonner.toast on copy — the module isn't installed
// in the test env's DOM, so mock to no-op.
vi.mock("sonner", () => ({ toast: toastFns }));

const CREATED_KEY: CreatedKey = {
  id: "svc-1",
  slug: "openai",
  catalogSlug: "openai",
  serviceName: "OpenAI",
  completionMode: "credential",
};

const MINTED_KEY: ApiKeyCreateResponse = {
  id: "ak-1",
  name: "Key for OpenAI",
  key_prefix: "nyxid_ag_abcd",
  full_key: "nyxid_ag_abcd_full_secret_value",
  scopes: ["proxy"],
  allow_all_services: false,
  allowed_service_ids: ["svc-1"],
  allow_all_nodes: true,
  allowed_node_ids: [],
  created_at: "2026-01-01T00:00:00Z",
  rate_limit_per_second: null,
  rate_limit_burst: null,
  platform: null,
} as unknown as ApiKeyCreateResponse;

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(window, "fetch").mockReset();
});

describe("ConnectVerifyStep — initial phase (connected)", () => {
  it("renders the 'you need an Agent Key' prompt and both CTAs", () => {
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    // Copy check: the "you need an Agent Key" prompt spans multiple
    // inline <span>s, so bounce off the aggregated body text rather
    // than any single element.
    expect(document.body.textContent ?? "").toMatch(
      /need an\s+Agent Key\s+to call/i,
    );
    expect(
      screen.getByRole("button", { name: /Create Agent Key/i }),
    ).toBeEnabled();
    expect(screen.getByRole("button", { name: /Maybe later/i })).toBeEnabled();
  });

  it("REGRESSION: clicking 'Create Agent Key' fires the mutation with proxy-scope + allowed_service_ids", async () => {
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    // The whole point of this test file: the click MUST invoke the
    // mutation. A silent-CTA regression here means the dispatch helper
    // has drifted back into "return the function reference" form.
    expect(createApiKeyMutate).toHaveBeenCalledTimes(1);
    expect(createApiKeyMutate).toHaveBeenCalledWith(
      {
        name: "Key for OpenAI",
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: ["svc-1"],
        allow_all_nodes: true,
      },
      expect.objectContaining({
        onSuccess: expect.any(Function),
        onError: expect.any(Function),
      }),
    );
  });

  it("'Maybe later' calls onDone without firing the mutation", async () => {
    const onDone = vi.fn();
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={onDone}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Maybe later/i }));

    expect(onDone).toHaveBeenCalledTimes(1);
    expect(createApiKeyMutate).not.toHaveBeenCalled();
  });
});

describe("ConnectVerifyStep — key_ready phase", () => {
  it("renders the AgentKeyPanel with Test / Wire buttons after mint succeeds", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    // Panel appears with the minted secret rendered verbatim (needed —
    // the raw secret is only shown once).
    await waitFor(() =>
      expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: /I'll wire it myself/i }),
    ).toBeEnabled();
  });

  it("mint failure surfaces the server error message + a Try again button", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onError?.(new Error("scoped-service-must-be-active"));
    });
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    await waitFor(() =>
      expect(
        screen.getByText(/scoped-service-must-be-active/i),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Try again/i })).toBeEnabled();
  });
});

describe("ConnectVerifyStep — probe phase", () => {
  it("Test click hits the proxy with Bearer auth and transitions to probe_success on 200", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const fetchMock = vi
      .spyOn(window, "fetch")
      .mockResolvedValue(new Response("{}", { status: 200 }));

    const succeededSpy = vi.fn();
    window.addEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [url, init] = fetchMock.mock.calls[0];
    expect(String(url)).toMatch(/\/api\/v1\/proxy\/s\/openai\/v1\/models$/);
    expect(init).toMatchObject({
      method: "GET",
      headers: expect.objectContaining({
        Authorization: `Bearer ${MINTED_KEY.full_key}`,
      }),
    });

    await waitFor(() =>
      expect(screen.getByText(/End-to-end test succeeded/i)).toBeInTheDocument(),
    );
    expect(succeededSpy).toHaveBeenCalledTimes(1);

    window.removeEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);
  });

  it("probe 403 renders the credential-rejected diagnose message + Retry test button", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    vi.spyOn(window, "fetch").mockResolvedValue(
      new Response("Forbidden", { status: 403 }),
    );

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(
        screen.getByText(/downstream rejected the bearer-authenticated call/i),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Retry test/i })).toBeEnabled();
    // The panel stays visible so the user can still copy the key even if
    // the probe fails — the failure is diagnostic, not blocking.
    expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument();
  });

  it("Test click dispatches loading start + end events regardless of probe outcome", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    vi.spyOn(window, "fetch").mockRejectedValue(new TypeError("network error"));

    const startSpy = vi.fn();
    const endSpy = vi.fn();
    window.addEventListener(VERIFY_KEY_LOADING_START_EVENT, startSpy);
    window.addEventListener(VERIFY_KEY_LOADING_END_EVENT, endSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() => expect(endSpy).toHaveBeenCalledTimes(1));
    expect(startSpy).toHaveBeenCalledTimes(1);

    window.removeEventListener(VERIFY_KEY_LOADING_START_EVENT, startSpy);
    window.removeEventListener(VERIFY_KEY_LOADING_END_EVENT, endSpy);
  });
});

describe("ConnectVerifyStep — Done and umbrella copy", () => {
  it("shell env snippet renders only on probe_success for openai-shaped slugs", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    vi.spyOn(window, "fetch").mockResolvedValue(
      new Response("{}", { status: 200 }),
    );

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
    );

    // Not visible before the probe runs. CopyableField renders a
    // <p>{label}</p>, so we assert on the label — matching the raw
    // multi-line value inside <code> is brittle.
    expect(screen.queryByText("Shell env")).toBeNull();

    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(screen.getByText("Shell env")).toBeInTheDocument(),
    );
    // The snippet's <code> has the two env-var lines — confirm they
    // rendered with the minted secret, not template placeholders.
    const envCode = document.body.textContent ?? "";
    expect(envCode).toMatch(/OPENAI_API_KEY="nyxid_ag_abcd_full_secret_value"/);
    expect(envCode).toMatch(/OPENAI_BASE_URL=/);
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
  });

  it("Done in probe_success calls onDone", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    vi.spyOn(window, "fetch").mockResolvedValue(
      new Response("{}", { status: 200 }),
    );

    const onDone = vi.fn();
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={onDone}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /^Done$/i }));

    expect(onDone).toHaveBeenCalledTimes(1);
  });
});
