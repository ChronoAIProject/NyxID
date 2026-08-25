import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PreviewAuthDeviceResponse } from "@/schemas/auth-device";
import { LoginDevicePage } from "./login-device";

const {
  approveMutate,
  denyMutate,
  navigate,
  previewMutate,
  previewReset,
  previewState,
} = vi.hoisted(() => ({
  approveMutate: vi.fn(),
  denyMutate: vi.fn(),
  navigate: vi.fn(),
  previewMutate: vi.fn(),
  previewReset: vi.fn(),
  previewState: {
    data: undefined as PreviewAuthDeviceResponse | undefined,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { readonly children: React.ReactNode }) => (
    <a href="/">{children}</a>
  ),
  useNavigate: () => navigate,
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({ isAuthenticated: true, isLoading: false }),
}));

vi.mock("@/hooks/use-auth-device", () => ({
  usePreviewAuthDevice: () => ({
    data: previewState.data,
    error: null,
    isError: false,
    isPending: false,
    mutateAsync: previewMutate,
    reset: previewReset,
  }),
  useApproveAuthDevice: () => ({
    isPending: false,
    mutateAsync: approveMutate,
    reset: vi.fn(),
  }),
  useDenyAuthDevice: () => ({
    isPending: false,
    mutateAsync: denyMutate,
    reset: vi.fn(),
  }),
}));

function makePreview(
  overrides: Partial<PreviewAuthDeviceResponse> = {},
): PreviewAuthDeviceResponse {
  return {
    client_label: "workstation",
    client_user_agent: "nyxid-cli/1.4.2 (macos; aarch64)",
    client_ip: "203.0.113.10",
    client_ip_attribution: "verified",
    client_country: "SG",
    client_kind: "cli",
    client_app: "NyxID CLI 1.4.2",
    client_platform: "macOS (aarch64)",
    same_ip_as_viewer: true,
    seconds_remaining: 600,
    initiated_at: "2099-08-20T10:00:00Z",
    expires_at: "2099-08-20T10:10:00Z",
    status: "pending",
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  previewState.data = undefined;
  previewMutate.mockResolvedValue(makePreview());
});

afterEach(() => {
  vi.useRealTimers();
});

describe("LoginDevicePage", () => {
  it("makes no request on mount, focus, or typing and previews only on Continue", async () => {
    const user = userEvent.setup();
    render(<LoginDevicePage />);
    const input = screen.getByLabelText("User code");

    expect(previewMutate).not.toHaveBeenCalled();
    await user.click(input);
    await user.type(input, "ABCD-EFGH");
    expect(previewMutate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(previewMutate).toHaveBeenCalledTimes(1);
    expect(previewMutate).toHaveBeenCalledWith("ABCDEFGH");
  });

  it("separates NyxID-verified facts from unverified device claims", async () => {
    const user = userEvent.setup();
    previewState.data = makePreview();
    render(<LoginDevicePage />);

    expect(screen.getByText("Verified by NyxID")).toBeInTheDocument();
    expect(
      screen.getByText("Reported by the requesting device (unverified)"),
    ).toBeInTheDocument();
    expect(screen.getByText("203.0.113.10 (SG)")).toBeInTheDocument();
    expect(screen.getByText("Same IP as this device")).toBeInTheDocument();
    expect(screen.getByText("NyxID CLI 1.4.2")).toBeInTheDocument();
    expect(screen.getByText("macOS (aarch64)")).toBeInTheDocument();

    const rawDetails = screen.getByText("Raw user agent").closest("details");
    expect(rawDetails).not.toHaveAttribute("open");
    await user.click(screen.getByText("Raw user agent"));
    expect(rawDetails).toHaveAttribute("open");
  });

  it("uses warning styling when the preview caller is on a different IP", () => {
    previewState.data = makePreview({ same_ip_as_viewer: false });
    render(<LoginDevicePage />);

    const signal = screen.getByText("Different IP from this device");
    expect(signal).toHaveClass("text-warning");
  });

  it("never presents an unavailable infrastructure IP as evidence", () => {
    previewState.data = makePreview({
      client_ip: "10.2.10.22",
      client_ip_attribution: "unavailable",
      client_country: null,
      same_ip_as_viewer: true,
    });
    render(<LoginDevicePage />);

    expect(
      screen.getByText("Requester IP is not available on this deployment."),
    ).toBeInTheDocument();
    expect(screen.queryByText("10.2.10.22")).not.toBeInTheDocument();
    expect(screen.queryByText("Same IP as this device")).not.toBeInTheDocument();
  });

  it("places an unverified reported IP with the requesting device claims", () => {
    previewState.data = makePreview({
      client_ip: "8.8.8.8",
      client_ip_attribution: "unverified",
      client_country: null,
      same_ip_as_viewer: null,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText("No requester IP was verified by NyxID.")).toBeInTheDocument();
    expect(screen.getByText("Reported IP (unverified)")).toBeInTheDocument();
    expect(screen.getByText("8.8.8.8")).toBeInTheDocument();
  });

  it("ticks to an expired panel, clears the decision actions, and stops at zero", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    previewState.data = makePreview({
      initiated_at: "2026-08-20T09:59:28Z",
      expires_at: "2026-08-20T10:00:02Z",
      seconds_remaining: 2,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText("32 seconds ago")).toBeInTheDocument();
    expect(screen.getByText("Expires in 0:02")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    expect(screen.getByText("Request expired")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reject" })).not.toBeInTheDocument();
    expect(screen.queryByText(/Expires in -/)).not.toBeInTheDocument();
  });

  it("offers decisions for a pending preview", () => {
    previewState.data = makePreview();
    render(<LoginDevicePage />);

    expect(screen.getByRole("button", { name: "Approve" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reject" })).toBeInTheDocument();
  });

  it.each([
    ["denied", "This login request was already denied."],
    ["expired", "This code has expired."],
    ["approved", "This code was already used."],
    ["delivered", "This code was already used."],
  ] as const)(
    "does not offer decisions for a %s preview",
    (status, expectedMessage) => {
      previewState.data = makePreview({ status });
      render(<LoginDevicePage />);

      expect(screen.getByText(expectedMessage)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Reject" })).not.toBeInTheDocument();
    },
  );
});
