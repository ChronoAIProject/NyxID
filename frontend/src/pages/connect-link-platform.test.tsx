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
import { ConnectLinkPage } from "./connect-link";
import { ConnectLinkContent } from "@/components/connect-link/connect-link-content";
import type { CompleteConnectLinkResponse } from "@/schemas/connect-links";

const mocks = vi.hoisted(() => ({
  complete: vi.fn(),
  preview: vi.fn(),
  navigate: vi.fn(),
  available: true,
  choice: undefined as boolean | undefined,
  scopes: [] as string[],
  endpointUrl: null as string | null,
  requiresGatewayUrl: false,
  authKeyName: "Authorization",
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ token: "hosted-token" }),
  useNavigate: () => mocks.navigate,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({
    isAuthenticated: true,
    isLoading: false,
    user: { email: "person@example.test" },
  }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalogEntry: () => ({
    data: {
      platform_key: { available: mocks.available, pricing: null },
      byok_pricing: null,
    },
  }),
}));
vi.mock("@/hooks/use-connect-links", () => ({
  usePreviewConnectLink: () => ({
    mutateAsync: mocks.preview,
    isPending: false,
    data: {
      status: "pending",
      service_name: "xAI",
      service_slug: "llm-xai",
      scopes: mocks.scopes,
      requested_by: "cli",
      expires_at: "2099-01-01T00:00:00Z",
      connect_method: "api_key",
      auth_key_name: mocks.authKeyName,
      endpoint_url: mocks.endpointUrl,
      requires_gateway_url: mocks.requiresGatewayUrl,
      use_platform_key: mocks.choice,
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
  useCancelHostedConnectLink: () => ({ isPending: false }),
  useConnectLinkStatus: () => ({}),
  connectLinkStorageKey: (id: string) => id,
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.available = true;
  mocks.choice = undefined;
  mocks.scopes = [];
  mocks.endpointUrl = null;
  mocks.requiresGatewayUrl = false;
  mocks.authKeyName = "Authorization";
  mocks.preview.mockResolvedValue({});
  mocks.complete.mockResolvedValue({ status: "completed" });
  let now = 100_000;
  vi.spyOn(Date, "now").mockImplementation(() => (now += 1000));
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
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
  const popup = {
    location: { href: "", assign: vi.fn() },
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
      "nyxid-connect-authorization",
      expect.stringContaining("popup"),
    ),
  );
  expect(popup.location.assign).toHaveBeenCalledWith(
    "https://provider.example/authorize",
  );
  expect(
    screen.getByText(/Finish authorization in the provider window/),
  ).toBeInTheDocument();
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
