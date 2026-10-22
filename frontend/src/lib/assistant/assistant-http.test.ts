import { beforeEach, describe, expect, it, vi } from "vitest";
import { capture, identify, initTelemetry, reset as resetTelemetry } from "@/lib/telemetry";
import { useConsentStore } from "@/stores/consent-store";
import {
  assignAssistantResponseConversation,
  assistantHttp,
  assistantJson,
} from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { transitionAssistantIdentity } from "./identity";

vi.mock("@/lib/telemetry", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/telemetry")>();
  return {
    ...original,
    capture: vi.fn(),
  };
});

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const WIRE_LOG_ID = "d7dbbf38-a31c-4331-8ddb-13fda5a70d12";

describe("assistantHttp", () => {
  it("reports malformed successful JSON without forwarding its body", async () => {
    vi.mocked(capture).mockClear();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("private malformed body", { status: 200 })));
    await expect(assistantJson("/assistant/nyxagent/models")).rejects.toThrow();
    expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({
      error_code: "NYXBOT_PROTOCOL_ERROR", engine: "nyxagent", http_status: 200,
    }) });
    expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toContain("private");
  });

  it.each([
    ["insufficient_credits", "Add credits in Billing & Usage"],
    ["wallet_suspended", "Your billing wallet is suspended"],
    ["plan_entitlement_required", "Your plan does not include this service"],
    ["billing_not_configured", "Billing setup is incomplete"],
    ["billing_provider_unavailable", "billing service is unavailable"],
  ])("explains the known billing cause %s before a turn is admitted", async (code, copy) => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: code, message: "SECRET upstream diagnostic" }),
      { status: 402 },
    )));
    await expect(assistantHttp("/assistant/nyxagent/turns")).rejects.toMatchObject({
      status: 402, message: expect.stringContaining(copy),
    });
  });

  it.each([false, true])("shows a server trace without exposing upstream details (JSON: %s)", async (json) => {
    const traceId = "9ad6d0fa-bb8f-489e-8fdd-f8e34960921b";
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(
      json ? JSON.stringify({ error: "upstream_error", message: "private upstream diagnostic" }) : "<html>proxy failure</html>",
      { status: 502, headers: { "X-NyxID-Trace-Id": traceId } },
    )));
    await expect(assistantHttp("/assistant/nyxagent/turns")).rejects.toMatchObject({
      status: 502,
      message: `NyxBot is temporarily unavailable. Try again shortly.\nTrace ID: ${traceId}`,
    });
  });

  it("explains a network failure without inventing a server trace", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("network error")));
    await expect(assistantHttp("/assistant/nyxagent/turns")).rejects.toMatchObject({
      status: 0,
      message: "Connection interrupted. Check your connection and the chat before trying again; your message may have arrived.",
    });
  });

  it("preserves deliberate cancellation", async () => {
    const controller = new AbortController();
    controller.abort();
    const error = new DOMException("Cancelled", "AbortError");
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(error));
    await expect(assistantHttp("/assistant/nyxagent/turns", { signal: controller.signal })).rejects.toBe(error);
  });

  it("does not use an approval request_id as a support trace", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({
      error: "approval_required", request_id: "approval-request", message: "Approval required",
    }), { status: 403 })));
    await expect(assistantHttp("/assistant/nyxagent/turns")).rejects.toMatchObject({ message: "Approval required" });
  });

  it("preserves validation details from a real error response", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "validation_error", error_code: 400, message: "Invalid", details: { rule: "required", field: "name", line: 2, remediation: "Add a name" } }), { status: 400, headers: { "Content-Type": "application/json" } })));
    await expect(assistantHttp("/assistant/test")).rejects.toMatchObject({ errorResponse: { details: { rule: "required", field: "name", line: 2 } } });
  });

  beforeEach(() => {
    vi.restoreAllMocks();
    vi.mocked(capture).mockClear();
    vi.unstubAllGlobals();
    globalThis.__nyxidAssistantHttpMock = undefined;
    resetTelemetry();
    useConsentStore.getState().bindAccount(null);
    useAssistantWireLogStore.setState({
      featureEnabled: false,
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
  });

  it.each([
    [403, "NYXBOT_HTTP_FORBIDDEN"],
    [500, "NYXBOT_HTTP_SERVER_ERROR"],
  ])(
    "records HTTP %s once with trace headers and no error body",
    async (status, code) => {
      vi.stubGlobal(
        "fetch",
        vi
          .fn()
          .mockResolvedValue(
            new Response(
              JSON.stringify({
                error: "failure",
                error_code: 9100,
                message: "private error details",
              }),
              {
                status,
                headers: {
                  "x-trace-id": "trace-123",
                  "x-request-id": "request-123",
                },
              },
            ),
          ),
      );

      await expect(
        assistantHttp("/assistant/chat", { method: "POST" }),
      ).rejects.toBeInstanceOf(ApiError);
      expect(capture).toHaveBeenCalledExactlyOnceWith({
        name: "nyxbot.error",
        props: expect.objectContaining({
          error_code: code,
          http_status: status,
          trace_id: "trace-123",
          request_id: "request-123",
          upstream_error_code: 9100,
        }),
      });
      expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toContain(
        "private error details",
      );
    },
  );

  it("lets the turn observer report before a 401 clears identity, without duplicate transport events", async () => {
    const order: string[] = [];
    useAuthStore.setState({
      setUser: () => {
        order.push("clear");
      },
    });
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          new Response(
            JSON.stringify({
              error: "session_dead",
              error_code: 1001,
              message: "Sign in",
            }),
            { status: 401 },
          ),
        ),
    );
    await expect(
      assistantHttp("/assistant/chat", {
        reportErrors: false,
        onError: () => {
          order.push("report");
        },
      }),
    ).rejects.toBeInstanceOf(ApiError);
    expect(order).toEqual(["report", "clear"]);
    expect(capture).not.toHaveBeenCalled();
  });

  it("records network failures but excludes aborted requests", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockRejectedValue(new TypeError("Failed to fetch")),
    );
    await expect(assistantHttp("/assistant/conversations")).rejects.toThrow();
    expect(capture).toHaveBeenCalledExactlyOnceWith({
      name: "nyxbot.error",
      props: expect.objectContaining({ error_code: "NYXBOT_NETWORK_ERROR" }),
    });
    vi.mocked(capture).mockClear();
    const controller = new AbortController();
    controller.abort();
    await expect(
      assistantHttp("/assistant/conversations", { signal: controller.signal }),
    ).rejects.toThrow();
    expect(capture).not.toHaveBeenCalled();
  });

  it("uses cookie auth and conditionally identifies the UI client", async () => {
    useAuthStore.setState({ user: { id: ACTOR_ID } as User });
    useConsentStore.getState().bindAccount(ACTOR_ID);
    useConsentStore.getState().setConsent(true);
    identify(ACTOR_ID);
    initTelemetry({ dsn: "test-fixture", consent: true });
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/assistant/conversations",
      expect.objectContaining({ credentials: "include" }),
    );
    expect(fetchMock.mock.calls[0]?.[1]?.headers).toMatchObject({
      "X-NyxID-Client": "ui",
      "X-NyxID-Analytics": "enabled",
      "X-NyxID-Analytics-Account": ACTOR_ID,
    });
  });

  it("preserves the session for an unattributed upstream 401", async () => {
    const setUser = vi.fn();
    useAuthStore.setState({ setUser });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ code: "UNAUTHORIZED", message: "No" }), {
          status: 401,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );

    await expect(assistantHttp("/assistant/chat")).rejects.toMatchObject({
      status: 401,
      errorCode: -1,
    });
    expect(setUser).not.toHaveBeenCalled();
  });

  it.each([1001, 2000, 2001, 2002])(
    "clears the session for attributed NyxID auth code %s",
    async (errorCode) => {
      const setUser = vi.fn();
      useAuthStore.setState({ setUser });
      vi.stubGlobal(
        "fetch",
        vi.fn().mockResolvedValue(
          new Response(
            JSON.stringify({
              error: "session_dead",
              error_code: errorCode,
              message: "Sign in again",
            }),
            { status: 401, headers: { "Content-Type": "application/json" } },
          ),
        ),
      );

      await expect(assistantHttp("/assistant/chat")).rejects.toBeInstanceOf(
        ApiError,
      );
      expect(setUser).toHaveBeenCalledWith(null);
    },
  );

  it("adds debug capture only for non-list assistant requests", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");
    await assistantHttp(`/assistant/conversations/${ACTOR_ID}/state`);

    expect(fetchMock.mock.calls[0]?.[1]?.headers).not.toHaveProperty(
      "X-NyxID-Debug-Upstream",
    );
    expect(fetchMock.mock.calls[1]?.[1]?.headers).toMatchObject({
      "X-NyxID-Debug-Upstream": "1",
    });
  });

  it("captures an id-backed response and assigns its adopted conversation", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response('data: {"type":"RUN_FINISHED"}\n\n', {
          status: 200,
          headers: {
            "Content-Type": "text/event-stream",
            "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID,
          },
        }),
      ),
    );

    const response = await assistantHttp("/assistant/chat", { method: "POST" });
    assignAssistantResponseConversation(response, ACTOR_ID);

    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
        wireLogId: WIRE_LOG_ID,
        conversationId: ACTOR_ID,
        label: "POST /assistant/chat",
        capture: {
          state: "settled",
          outcome: "complete",
          body: { truncated: false },
        },
      });
    });
  });

  it("never captures list or wire-log retrieval responses recursively", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("{}", {
        status: 200,
        headers: { "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await assistantHttp("/assistant/conversations");
    await assistantHttp(`/assistant/wire-logs/${WIRE_LOG_ID}`);

    for (const call of fetchMock.mock.calls) {
      expect(call[1]?.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
    }
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });
});


function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("originating-account telemetry", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.mocked(capture).mockClear();
    globalThis.__nyxidAssistantHttpMock = undefined;
    transitionAssistantIdentity("account-a");
    useAssistantWireLogStore.setState({ featureEnabled: false, captureEnabled: false });
  });

  it.each([false, true])("does not report a late HTTP failure after an account transition (ABA: %s)", async (aba) => {
    const response = deferred<Response>();
    const fetchMock = vi.fn().mockReturnValue(response.promise);
    vi.stubGlobal("fetch", fetchMock);
    const pending = expect(assistantHttp("/assistant/nyxagent/conversations")).rejects.toMatchObject({ status: 403 });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    transitionAssistantIdentity("account-b");
    if (aba) transitionAssistantIdentity("account-a");
    response.resolve(new Response("{}", { status: 403 }));
    await pending;
    expect(capture).not.toHaveBeenCalled();
  });

  it.each([false, true])("does not report late invalid JSON after an account transition (ABA: %s)", async (aba) => {
    const body = deferred<unknown>();
    const response = new Response("{}", { status: 200 });
    const parse = vi.spyOn(response, "json").mockImplementation(async () => {
      await body.promise;
      throw new SyntaxError("private invalid response");
    });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response));
    const pending = expect(assistantJson("/assistant/nyxagent/conversations")).rejects.toThrow(SyntaxError);
    await vi.waitFor(() => expect(parse).toHaveBeenCalledOnce());
    transitionAssistantIdentity("account-b");
    if (aba) transitionAssistantIdentity("account-a");
    body.resolve(null);
    await pending;
    expect(capture).not.toHaveBeenCalled();
  });
});

function ownerAuthority() {
  useAuthStore.setState({ user: { id: "owner-one" } as User });
  const accountChanged = new Error(
    "Your account changed. Reload before managing this Machine.",
  );
  return {
    accountChanged,
    authorityGuard: () => {
      if (useAuthStore.getState().user?.id !== "owner-one") throw accountChanged;
    },
    switchAccount: () => {
      useAuthStore.setState({ user: { id: "owner-two" } as User });
    },
  };
}

describe("assistant transport authority", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    globalThis.__nyxidAssistantHttpMock = undefined;
    useAssistantWireLogStore.setState({ featureEnabled: false, captureEnabled: false });
  });

  it("rejects stale authority before calling either transport", async () => {
    const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
    const mocked = vi.fn();
    const transport = vi.fn();
    globalThis.__nyxidAssistantHttpMock = mocked;
    vi.stubGlobal("fetch", transport);
    switchAccount();

    await expect(assistantHttp("/assistant/test", { authorityGuard })).rejects.toBe(accountChanged);

    expect(mocked).not.toHaveBeenCalled();
    expect(transport).not.toHaveBeenCalled();
  });

  it.each([false, true])(
    "rejects an account switch during the mock seam without sending a fetch (mocked: %s)",
    async (hasMockResponse) => {
      const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
      const response = deferred<Response | undefined>();
      globalThis.__nyxidAssistantHttpMock = () => response.promise;
      const transport = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
      vi.stubGlobal("fetch", transport);
      const pending = assistantHttp("/assistant/test", { authorityGuard });
      const rejected = expect(pending).rejects.toBe(accountChanged);

      switchAccount();
      response.resolve(hasMockResponse ? new Response(null, { status: 204 }) : undefined);

      await rejected;
      expect(transport).not.toHaveBeenCalled();
    },
  );

  it("rejects a successful HTTP response after the account switches during fetch", async () => {
    const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
    const response = deferred<Response>();
    const transport = vi.fn().mockReturnValue(response.promise);
    vi.stubGlobal("fetch", transport);
    const pending = assistantHttp("/assistant/test", { authorityGuard });
    const rejected = expect(pending).rejects.toBe(accountChanged);
    await vi.waitFor(() => expect(transport).toHaveBeenCalledOnce());

    switchAccount();
    response.resolve(new Response(null, { status: 204 }));

    await rejected;
  });

  it("rejects parsed JSON after the account switches during body parsing", async () => {
    const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
    const body = deferred<unknown>();
    const response = new Response("{}", { status: 200 });
    const parse = vi.spyOn(response, "json").mockReturnValue(body.promise);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response));
    const pending = assistantJson("/assistant/test", { authorityGuard });
    const rejected = expect(pending).rejects.toBe(accountChanged);
    await vi.waitFor(() => expect(parse).toHaveBeenCalledOnce());

    switchAccount();
    body.resolve({ private: "owner-one" });

    await rejected;
  });

  it("does not clear the new account when an old request returns a late 401", async () => {
    const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
    const response = deferred<Response>();
    const transport = vi.fn().mockReturnValue(response.promise);
    const setUser = vi.fn();
    useAuthStore.setState({ setUser });
    vi.stubGlobal("fetch", transport);
    const pending = assistantHttp("/assistant/test", { authorityGuard });
    const rejected = expect(pending).rejects.toBe(accountChanged);
    await vi.waitFor(() => expect(transport).toHaveBeenCalledOnce());

    switchAccount();
    response.resolve(new Response(JSON.stringify({
      error: "session_dead", error_code: 1001, message: "Sign in again",
    }), { status: 401 }));

    await rejected;
    expect(setUser).not.toHaveBeenCalled();
    expect(useAuthStore.getState().user?.id).toBe("owner-two");
  });

  it("does not clear the new account when it switches during 401 body parsing", async () => {
    const { authorityGuard, accountChanged, switchAccount } = ownerAuthority();
    const body = deferred<unknown>();
    const response = new Response("{}", { status: 401 });
    const parse = vi.spyOn(response, "json").mockReturnValue(body.promise);
    const setUser = vi.fn();
    useAuthStore.setState({ setUser });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response));
    const pending = assistantHttp("/assistant/test", { authorityGuard });
    const rejected = expect(pending).rejects.toBe(accountChanged);
    await vi.waitFor(() => expect(parse).toHaveBeenCalledOnce());

    switchAccount();
    body.resolve({ error: "session_dead", error_code: 1001, message: "Sign in again" });

    await rejected;
    expect(setUser).not.toHaveBeenCalled();
    expect(useAuthStore.getState().user?.id).toBe("owner-two");
  });
});
