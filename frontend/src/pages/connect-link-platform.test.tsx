import { StrictMode, useCallback, useState } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ConnectLinkPage, ConnectLinkReturnPage } from "./connect-link";
import { ConnectLinkContent } from "@/components/connect-link/connect-link-content";
import type { CompleteConnectLinkResponse } from "@/schemas/connect-links";

const mocks = vi.hoisted(() => ({
  complete: vi.fn(),
  preview: vi.fn(),
  navigate: vi.fn(),
  terminal: null as "completed" | "cancelled" | "expired" | null,
  callback: "desktop-app://return?state=123",
  cancel: vi.fn(),
  actor: "person" as string | undefined,
  token: "hosted-token",
  available: true,
  intentLocked: false,
  choice: undefined as boolean | undefined,
  scopes: [] as string[],
  endpointUrl: null as string | null,
  requiresGatewayUrl: false,
  authKeyName: "Authorization",
  connectMethod: "api_key",
  catalogUnavailable: false,
  fields: null as { name: string; label: string; secret: boolean }[] | null,
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ token: mocks.token, linkId: "connect-link-1" }),
  useNavigate: () => mocks.navigate,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({
    isAuthenticated: !!mocks.actor,
    isLoading: false,
    user: mocks.actor ? { id: mocks.actor, email: "person@example.test" } : null,
  }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalogEntry: () => ({
    data: mocks.catalogUnavailable
      ? undefined
      : {
          platform_key: { available: mocks.available, pricing: null },
          byok_pricing: null,
          token_exchange_credential_fields: mocks.fields,
        },
  }),
}));
vi.mock("@/hooks/use-connect-links", () => ({
  usePreviewConnectLink: () => ({
    mutateAsync: mocks.preview,
    isPending: false,
    data: {
      status: mocks.terminal ?? "pending",
      callback_url: mocks.terminal ? mocks.callback : null,
      service_name: "xAI",
      service_slug: "llm-xai",
      scopes: mocks.scopes,
      requested_by: "cli",
      expires_at: "2099-01-01T00:00:00Z",
      connect_method: mocks.connectMethod,
      auth_key_name: mocks.authKeyName,
      token_exchange_credential_fields: mocks.catalogUnavailable
        ? mocks.fields
        : undefined,
      endpoint_url: mocks.endpointUrl,
      requires_gateway_url: mocks.requiresGatewayUrl,
      use_platform_key: mocks.choice,
      assistant_intent_locked: mocks.intentLocked,
    },
  }),
  useCompleteConnectLink: () => {
    const [data, setData] = useState<CompleteConnectLinkResponse>();
    const mutateAsync = useCallback(async (input: unknown) => {
      const result = (await mocks.complete(
        input,
      )) as CompleteConnectLinkResponse;
      setData(result);
      return result;
    }, []);
    return { mutateAsync, isPending: false, data };
  },
  useCancelHostedConnectLink: () => {
    const [data, setData] = useState();
    return {
      data,
      isPending: false,
      mutateAsync: async (token: string) => {
        const result = await mocks.cancel(token);
        setData(result);
        return result;
      },
    };
  },
  useConnectLinkStatus: () => ({
    data: { id: "connect-link-1", status: mocks.terminal ?? "pending", callback_url: mocks.callback },
  }),
  connectLinkStorageKey: (id: string) => id,
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.terminal = null;
  mocks.available = true;
  mocks.actor = "person";
  mocks.token = "hosted-token";
  mocks.intentLocked = false;
  mocks.choice = undefined;
  mocks.scopes = [];
  mocks.endpointUrl = null;
  mocks.requiresGatewayUrl = false;
  mocks.authKeyName = "Authorization";
  mocks.connectMethod = "api_key";
  mocks.fields = null;
  mocks.catalogUnavailable = false;
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
  mocks.preview.mockResolvedValue({});
  mocks.complete.mockResolvedValue({ status: "completed" });
  let now = 100_000;
  vi.spyOn(Date, "now").mockImplementation(() => (now += 1000));
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});
it.each(["another-person", undefined])(
  "clears entered credentials and refreshes the preview when the account becomes %s",
  async (actor) => {
    mocks.available = false;
    const view = render(<ConnectLinkPage />);
    const credential = await screen.findByPlaceholderText(
      "Paste API key or token for xAI",
    );
    await userEvent.type(credential, "private-entered-credential");
    await waitFor(() => expect(mocks.preview).toHaveBeenCalledTimes(1));
    mocks.actor = actor;
    view.rerender(<ConnectLinkPage />);
    expect(credential).not.toBeInTheDocument();
    expect(
      screen.queryByDisplayValue("private-entered-credential"),
    ).not.toBeInTheDocument();
    if (actor) {
      await waitFor(() => expect(mocks.preview).toHaveBeenCalledTimes(2));
      expect(
        screen.getByPlaceholderText("Paste API key or token for xAI"),
      ).toHaveValue("");
    } else {
      expect(mocks.navigate).toHaveBeenCalledWith({
        to: "/login",
        search: { return_to: `${window.location.origin}/connect/hosted-token` },
      });
    }
    expect(mocks.complete).not.toHaveBeenCalled();
  },
);
it("clears entered credentials when opening a different connection request", async () => {
  mocks.available = false;
  const view = render(<ConnectLinkPage />);
  const credential = await screen.findByPlaceholderText(
    "Paste API key or token for xAI",
  );
  await userEvent.type(credential, "private-entered-credential");
  mocks.token = "another-hosted-token";
  view.rerender(<ConnectLinkPage />);
  expect(credential).not.toBeInTheDocument();
  expect(
    screen.getByPlaceholderText("Paste API key or token for xAI"),
  ).toHaveValue("");
  await waitFor(() =>
    expect(mocks.preview).toHaveBeenLastCalledWith("another-hosted-token"),
  );
  expect(mocks.complete).not.toHaveBeenCalled();
});
it("defaults to platform and completes without any secret", async () => {
  render(<ConnectLinkPage />);
  expect(
    screen.getByRole("heading", { name: "NyxID wants to connect to your xAI" }),
  ).toBeInTheDocument();
  await waitFor(() =>
    expect(mocks.preview).toHaveBeenCalledWith("hosted-token"),
  );
  expect(screen.getByRole("radio", { name: /Use NyxID's key/ })).toBeChecked();
  await userEvent.click(
    screen.getByRole("button", { name: "Approve connection" }),
  );
  expect(mocks.complete).toHaveBeenCalledWith({
    token: "hosted-token",
    values: { use_platform_key: true },
  });
});

it("keeps embedded OAuth authorization in a provider popup", async () => {
  mocks.connectMethod = "oauth";
  const setItem = vi.fn();
  const assign = vi.fn(() => {
    expect(setItem).toHaveBeenCalledWith("connect-link-1", "hosted-token");
  });
  const popup = {
    sessionStorage: { setItem },
    location: { href: "", assign },
    close: vi.fn(),
  } as unknown as Window;
  const open = vi.spyOn(window, "open").mockReturnValue(popup);
  mocks.complete.mockResolvedValueOnce({
    status: "oauth_required",
    id: "connect-link-1",
    authorization_url: "https://provider.example/authorize",
  });

  render(
    <ConnectLinkContent
      token="hosted-token"
      embedded
      redirectOnTerminal={false}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve connection" }),
  );

  await waitFor(() =>
    expect(open).toHaveBeenCalledWith(
      "about:blank",
      "_blank",
      expect.stringContaining("popup"),
    ),
  );
  expect(popup.location.assign).toHaveBeenCalledWith(
    "https://provider.example/authorize",
  );
  expect(
    screen.getByText(/Finish authorization in the provider window/),
  ).toBeInTheDocument();
  expect(sessionStorage.getItem("connect-link-1")).toBeNull();
});

it("does not reopen an authorization popup if setup closes during the request", async () => {
  mocks.connectMethod = "oauth";
  let finish!: (result: CompleteConnectLinkResponse) => void;
  mocks.complete.mockImplementationOnce(
    () =>
      new Promise<CompleteConnectLinkResponse>((resolve) => {
        finish = resolve;
      }),
  );
  const popup = {
    sessionStorage: { setItem: vi.fn() },
    location: { assign: vi.fn() },
    close: vi.fn(),
  } as unknown as Window;
  const open = vi.spyOn(window, "open").mockReturnValue(popup);
  const view = render(
    <ConnectLinkContent
      token="hosted-token"
      embedded
      redirectOnTerminal={false}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve connection" }),
  );
  expect(open).toHaveBeenCalledTimes(1);
  view.unmount();
  expect(popup.close).toHaveBeenCalledTimes(1);
  await act(async () =>
    finish({
      status: "oauth_required",
      id: "connect-link-1",
      service_slug: "llm-xai",
      authorization_url: "https://provider.example/authorize",
    }),
  );
  expect(open).toHaveBeenCalledTimes(1);
  expect(popup.location.assign).not.toHaveBeenCalled();
  expect(popup.sessionStorage.setItem).not.toHaveBeenCalled();
});

it("preserves the host theme when embedded content closes", () => {
  document.documentElement.classList.add("theme-light");
  const view = render(
    <ConnectLinkContent
      token="hosted-token"
      embedded
      redirectOnTerminal={false}
    />,
  );
  view.unmount();
  expect(document.documentElement).toHaveClass("theme-light");
  document.documentElement.classList.remove("theme-light");
});

it("previews once under React StrictMode", async () => {
  render(
    <StrictMode>
      <ConnectLinkPage />
    </StrictMode>,
  );
  await screen.findByRole("heading", {
    name: "NyxID wants to connect to your xAI",
  });
  await waitFor(() =>
    expect(mocks.preview).toHaveBeenCalledExactlyOnceWith("hosted-token"),
  );
});
it.each([false, true])(
  "submits an explicit own-key choice when the creator choice is %s and access is revoked",
  async (choice) => {
    mocks.choice = choice;
    mocks.available = false;
    render(<ConnectLinkPage />);
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Authorization")).toHaveAttribute(
      "placeholder",
      "Paste API key or token for xAI",
    );
    await userEvent.type(
      screen.getByLabelText("Authorization"),
      "personal-secret",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Approve & connect" }),
    );
    expect(mocks.complete).toHaveBeenCalledWith({
      token: "hosted-token",
      values: expect.objectContaining({
        use_platform_key: false,
        credential: "personal-secret",
      }),
    });
  },
);
it("retains creator-requested OAuth scope setup without defaulting to a platform key", async () => {
  mocks.scopes = ["read:org"];
  render(<ConnectLinkPage />);
  expect(screen.queryByRole("radio")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Authorization")).toBeInTheDocument();
  expect(mocks.complete).not.toHaveBeenCalled();
});

it("prefills an editable gateway URL and submits it with the credential", async () => {
  mocks.available = false;
  mocks.requiresGatewayUrl = true;
  mocks.authKeyName = "";
  mocks.endpointUrl = "https://gateway.example.test";
  render(<ConnectLinkPage />);
  const url = screen.getByRole("textbox", { name: "Service URL" });
  expect(url).toHaveValue("https://gateway.example.test");
  expect(screen.getByLabelText("Gateway bearer token")).toHaveAttribute(
    "placeholder",
    "Paste bearer token for xAI",
  );
  await userEvent.clear(url);
  await userEvent.type(url, "https://another.example.test");
  await userEvent.type(
    screen.getByLabelText("Gateway bearer token"),
    "personal-secret",
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve & connect" }),
  );
  expect(mocks.complete).toHaveBeenCalledWith({
    token: "hosted-token",
    values: expect.objectContaining({
      endpoint_url: "https://another.example.test",
    }),
  });
});

it("collects declared token-exchange fields and submits them as one JSON credential", async () => {
  mocks.available = false;
  mocks.catalogUnavailable = true;
  mocks.fields = [
    { name: "app_id", label: "App ID", secret: false },
    { name: "app_secret", label: "App Secret", secret: true },
  ];
  render(<ConnectLinkPage />);
  const approve = screen.getByRole("button", { name: "Approve & connect" });
  await userEvent.type(screen.getByLabelText("App ID"), "cli_app");
  expect(approve).toBeDisabled();
  fireEvent.submit(approve.closest("form")!);
  await screen.findByText("Enter App Secret.");
  expect(mocks.complete).not.toHaveBeenCalled();
  expect(screen.getByLabelText("App Secret")).toHaveAttribute(
    "type",
    "password",
  );
  await userEvent.type(screen.getByLabelText("App Secret"), "app-secret");
  await userEvent.click(approve);
  expect(mocks.complete).toHaveBeenCalledWith({
    token: "hosted-token",
    values: expect.objectContaining({
      credential: JSON.stringify({
        app_id: "cli_app",
        app_secret: "app-secret",
      }),
    }),
  });
});

it("polls a provider device code at its server interval and displays completion", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-28T00:00:00Z"));
  mocks.complete
    .mockResolvedValueOnce({
      status: "device_code_required",
      device_user_code: "ABCD-EFGH",
      device_verification_uri: "https://provider.example/device",
      device_state: "device-state",
      device_interval: 5,
      device_status: "pending",
    })
    .mockResolvedValueOnce({
      status: "device_code_required",
      device_state: "device-state",
      device_interval: 10,
      device_status: "slow_down",
    })
    .mockResolvedValueOnce({ status: "completed", callback_url: null });

  render(<ConnectLinkPage />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Approve connection" }));
  });
  expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /Use NyxID's key/ })).toBeDisabled();
  expect(
    screen.getByText("Checking automatically every 5 seconds."),
  ).toBeInTheDocument();
  expect(mocks.complete).toHaveBeenCalledTimes(1);

  await act(async () => vi.advanceTimersByTimeAsync(5_000));
  expect(mocks.complete).toHaveBeenCalledTimes(2);
  expect(
    screen.getByText(
      "Provider requested a slower check. Checking again in 10 seconds.",
    ),
  ).toBeInTheDocument();
  await act(async () => vi.advanceTimersByTimeAsync(9_000));
  expect(mocks.complete).toHaveBeenCalledTimes(2);
  await act(async () => vi.advanceTimersByTimeAsync(1_000));
  expect(mocks.complete).toHaveBeenCalledTimes(3);
  expect(
    screen.getByRole("heading", { name: "xAI connected" }),
  ).toBeInTheDocument();
  expect(screen.getByText("Connection completed")).toBeInTheDocument();
  await act(async () => vi.advanceTimersByTimeAsync(20_000));
  expect(mocks.complete).toHaveBeenCalledTimes(3);
});

it("keeps retrying after a transient device poll failure without offering a new code", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-28T00:00:00Z"));
  mocks.complete
    .mockResolvedValueOnce({
      status: "device_code_required",
      device_user_code: "ABCD-EFGH",
      device_verification_uri: "https://provider.example/device",
      device_state: "device-state",
      device_interval: 5,
      device_status: "pending",
    })
    .mockRejectedValueOnce(new Error("Temporary network failure"))
    .mockResolvedValueOnce({ status: "completed", callback_url: null });

  render(<ConnectLinkPage />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Approve connection" }));
  });
  await act(async () => vi.advanceTimersByTimeAsync(5_000));
  expect(mocks.complete).toHaveBeenCalledTimes(2);
  expect(
    screen.queryByRole("button", { name: "Get a new code" }),
  ).not.toBeInTheDocument();
  await act(async () => vi.advanceTimersByTimeAsync(5_000));
  expect(mocks.complete).toHaveBeenCalledTimes(3);
  expect(
    screen.getByRole("heading", { name: "xAI connected" }),
  ).toBeInTheDocument();
});

it("hides credential switching and completes the stored assistant personal intent", async () => {
  mocks.intentLocked = true;
  mocks.choice = false;
  mocks.complete.mockResolvedValue({
    id: "id",
    status: "completed",
    service_slug: "llm-xai",
  });
  render(<ConnectLinkPage />);
  expect(screen.queryByText("Use platform key")).not.toBeInTheDocument();
  expect(screen.getByText(/fixed/i)).toBeVisible();
});

it("a saved personal API key is not presented as Connected", async () => {
  mocks.available = false;
  mocks.complete.mockResolvedValue({ status: "completed", credential_verification: { state: "rejected", reason_code: "provider_forbidden", can_check: true } });
  render(<ConnectLinkContent token="hosted-token" embedded redirectOnTerminal={false} />);
  await userEvent.type(screen.getByLabelText("Authorization"), "fixture-key");
  await userEvent.click(screen.getByRole("button", { name: "Approve & connect" }));
  expect(await screen.findByRole("heading", { name: "xAI saved" })).toBeInTheDocument();
  expect(screen.getByText("Rejected by provider")).toBeInTheDocument();
  expect(screen.getByText(/does not necessarily mean the key is invalid/)).toBeInTheDocument();
  expect(screen.queryByText("xAI connected")).toBeNull();
});

it.each(["completed", "cancelled", "expired"] as const)(
  "shows the %s state to a later viewer on both hosted routes",
  async (terminal) => {
    mocks.terminal = terminal;
    vi.useFakeTimers();
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    for (const Page of [ConnectLinkPage, ConnectLinkReturnPage]) {
      const view = render(<Page />);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1600);
      });
      expect(assign).not.toHaveBeenCalled();
      expect(screen.getByRole("status")).toBeInTheDocument();
      expect(
        screen.queryByText(/Returning to the requesting application/),
      ).toBeNull();
      view.unmount();
    }
  },
);

it("returns from an approved hosted connection only while that actor and link are open", async () => {
  vi.useFakeTimers();
  const assign = vi
    .spyOn(window.location, "assign")
    .mockImplementation(() => {});
  mocks.complete.mockResolvedValue({
    status: "completed",
    callback_url: mocks.callback,
  });
  const view = render(<ConnectLinkPage />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Approve connection" }));
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1500);
  });
  expect(assign).toHaveBeenCalledExactlyOnceWith(mocks.callback);
  mocks.actor = "other-person";
  mocks.terminal = "completed";
  view.rerender(<ConnectLinkPage />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1600);
  });
  expect(assign).toHaveBeenCalledTimes(1);
});

it.each([true, false])(
  "returns only from a decline applied in this flow (%s)",
  async (cancelledHere) => {
    vi.useFakeTimers();
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    mocks.cancel.mockResolvedValue({
      status: "cancelled",
      callback_url: mocks.callback,
      cancelled_here: cancelledHere,
    });
    render(<ConnectLinkPage />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    });
    expect(mocks.cancel).toHaveBeenCalledWith("hosted-token");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(assign.mock.calls).toEqual(cancelledHere ? [[mocks.callback]] : []);
  },
);

it.each(["same flow", "other account", "other attempt", "legacy callback"])(
  "correlates a hosted OAuth return with its tab and account (%s)",
  async (mode) => {
    vi.useFakeTimers();
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    const authorization = "https://provider.example/authorize?state=abc";
    mocks.connectMethod = "oauth";
    mocks.complete.mockResolvedValue({
      status: "oauth_required",
      id: "connect-link-1",
      authorization_url: authorization,
    });
    const hosted = render(<ConnectLinkPage />);
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Approve connection" }),
      );
    });
    expect(assign).toHaveBeenCalledWith(authorization);
    hosted.unmount();
    assign.mockClear();

    // SHA-256 of the synthetic OAuth state returned by the provider.
    const receipt =
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const fragment =
      mode === "legacy callback"
        ? ""
        : `#flow=${mode === "other attempt" ? "different" : receipt}`;
    window.history.replaceState(
      null,
      "",
      `/connect/return/connect-link-1?provider_status=success${fragment}`,
    );
    if (mode === "other account") mocks.actor = "other-person";
    mocks.terminal = "completed";
    const returned = render(
      <StrictMode>
        <ConnectLinkReturnPage />
      </StrictMode>,
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(assign.mock.calls).toEqual(
      mode === "same flow" ? [[mocks.callback]] : [],
    );
    if (mode === "legacy callback") {
      fireEvent.click(
        screen.getByRole("button", { name: "Return to application" }),
      );
      expect(assign).toHaveBeenCalledExactlyOnceWith(mocks.callback);
    }
    returned.unmount();
    assign.mockClear();
    render(<ConnectLinkReturnPage />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(assign).not.toHaveBeenCalled();
  },
);
