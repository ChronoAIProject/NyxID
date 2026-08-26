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
    client_city: "Singapore",
    client_region: "Singapore",
    client_continent: "AS",
    client_ip_timezone: "Asia/Singapore",
    initiating_origin: "https://nyxid.dev",
    initiating_origin_status: "matched",
    client_kind: "cli",
    client_app: "NyxID CLI 1.4.2",
    client_platform: "macOS (aarch64)",
    client_model: null,
    client_form_factor: null,
    client_timezone: null,
    client_timezone_matches_ip: null,
    client_locale: null,
    client_screen_width: null,
    client_screen_height: null,
    client_device_pixel_ratio: null,
    client_hardware_concurrency: null,
    client_device_memory: null,
    same_ip_as_viewer: true,
    network_relation: "same_ip",
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
  vi.restoreAllMocks();
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

  it("presents requester facts and device claims as neutral detail rows", async () => {
    const user = userEvent.setup();
    previewState.data = makePreview();
    render(<LoginDevicePage />);

    expect(screen.queryByText("Verified by NyxID")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Reported by the requesting device (unverified)"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("203.0.113.10")).toBeInTheDocument();
    expect(
      screen.getByText("Singapore, Singapore (SG) · Asia/Singapore"),
    ).toBeInTheDocument();
    const network = screen.getByText("Same IP as this device");
    expect(network).not.toHaveClass("text-success");
    expect(network).not.toHaveClass("text-warning");
    expect(screen.getByText("NyxID CLI 1.4.2")).toBeInTheDocument();
    expect(screen.getByText("macOS (aarch64)")).toBeInTheDocument();
    const normalExpiry = screen.getByText("Expires in").nextElementSibling;
    expect(normalExpiry).toBeInTheDocument();
    expect(normalExpiry).not.toHaveClass("text-warning");
    expect(normalExpiry).not.toHaveClass("text-destructive");
    expect(
      screen.getByText(/Only approve if you started this sign-in/),
    ).toBeInTheDocument();

    const rawDetails = screen.getByText("Raw user agent").closest("details");
    expect(rawDetails).not.toHaveAttribute("open");
    await user.click(screen.getByText("Raw user agent"));
    expect(rawDetails).toHaveAttribute("open");
  });

  it("never presents a matched Origin header as verified assurance", () => {
    previewState.data = makePreview({
      initiating_origin: "https://nyxid.dev",
      initiating_origin_status: "matched",
    });
    render(<LoginDevicePage />);

    expect(
      screen.queryByText(/Started from nyxid\.dev/i),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(/configured NyxID site/i),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Started from")).not.toBeInTheDocument();
  });

  it("presents different networks as useful context rather than an alarm", () => {
    previewState.data = makePreview({
      same_ip_as_viewer: false,
      network_relation: "different_network",
    });
    render(<LoginDevicePage />);

    const signal = screen.getByText("Different network");
    expect(signal).not.toHaveClass("text-warning");
    expect(signal).not.toHaveClass("text-destructive");
    expect(signal).not.toHaveClass("text-success");
  });

  it("shows rich browser recognition details and both timezone mismatch signals", () => {
    vi.spyOn(Intl.DateTimeFormat.prototype, "resolvedOptions").mockReturnValue({
      ...new Intl.DateTimeFormat().resolvedOptions(),
      timeZone: "Asia/Singapore",
    });
    previewState.data = makePreview({
      client_kind: "browser",
      client_label: "Chrome 131.0.6778.85 on macOS 15.2",
      client_app: "Chrome 131.0.6778.85",
      client_platform: "macOS 15.2 (arm64)",
      client_form_factor: "desktop",
      client_timezone: "Europe/Moscow",
      client_timezone_matches_ip: false,
      client_locale: "en-SG",
      client_screen_width: 1512,
      client_screen_height: 982,
      client_device_pixel_ratio: 2,
      client_hardware_concurrency: 12,
      client_device_memory: 16,
      network_relation: "same_network",
      same_ip_as_viewer: false,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText("Same network as this device")).toBeInTheDocument();
    const timezone = screen.getByText(
      "Europe/Moscow · differs from this device and IP location",
    );
    expect(timezone).toHaveClass("text-warning");
    expect(screen.getByText("1512 x 982 CSS px at 2x")).toBeInTheDocument();
    expect(screen.getByText("12 logical processors")).toBeInTheDocument();
    expect(screen.getByText("16 GB")).toBeInTheDocument();
  });

  it("prioritizes the origin anomaly when multiple signals differ", () => {
    previewState.data = makePreview({
      initiating_origin: "https://login-copy.example",
      initiating_origin_status: "mismatched",
      client_timezone: "Europe/Moscow",
      client_timezone_matches_ip: false,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText("login-copy.example")).toHaveClass(
      "text-destructive",
    );
    const timezone = screen.getByText(
      "Europe/Moscow · differs from this device and IP location",
    );
    expect(timezone).not.toHaveClass("text-warning");
    expect(timezone).not.toHaveClass("text-destructive");
  });

  it("tints only the mismatched initiating-origin value", () => {
    previewState.data = makePreview({
      initiating_origin: "https://login-copy.example",
      initiating_origin_status: "mismatched",
    });
    render(<LoginDevicePage />);

    const origin = screen.getByText("login-copy.example");
    expect(origin).toHaveClass("text-destructive");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it.each([
    ["malformed", "Malformed origin"],
    ["non_http", "Non-HTTP origin"],
  ] as const)("distinguishes an %s initiating origin", (status, message) => {
    previewState.data = makePreview({
      initiating_origin:
        status === "non_http" ? "file:///tmp/login.html" : "not a url",
      initiating_origin_status: status,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText(message)).toHaveClass("text-destructive");
  });

  it("keeps a CLI-shaped request neutral when browser context and origin are absent", () => {
    previewState.data = makePreview({
      initiating_origin: null,
      initiating_origin_status: "absent",
      client_city: null,
      client_region: null,
      client_continent: null,
      client_ip_timezone: null,
      client_timezone: null,
      client_timezone_matches_ip: null,
      client_locale: null,
      client_form_factor: null,
      client_screen_width: null,
      client_screen_height: null,
      client_device_pixel_ratio: null,
      client_hardware_concurrency: null,
      client_device_memory: null,
    });
    render(<LoginDevicePage />);

    expect(
      screen.queryByText(/not the official NyxID site/i),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/origin.*malformed/i)).not.toBeInTheDocument();
    expect(screen.getByText("NyxID CLI 1.4.2")).toBeInTheDocument();
  });

  it("never presents an unavailable infrastructure IP as evidence", () => {
    previewState.data = makePreview({
      client_ip: "10.2.10.22",
      client_ip_attribution: "unavailable",
      client_country: null,
      same_ip_as_viewer: true,
      network_relation: null,
    });
    render(<LoginDevicePage />);

    expect(
      screen.getByText("IP unavailable on this deployment"),
    ).toBeInTheDocument();
    expect(screen.queryByText("10.2.10.22")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Same IP as this device"),
    ).not.toBeInTheDocument();
  });

  it("places an unverified reported IP with the requesting device claims", () => {
    previewState.data = makePreview({
      client_ip: "8.8.8.8",
      client_ip_attribution: "unverified",
      client_country: null,
      same_ip_as_viewer: null,
      network_relation: null,
    });
    render(<LoginDevicePage />);

    expect(screen.getByText("Not verified")).toBeInTheDocument();
    expect(screen.getByText("Reported IP")).toBeInTheDocument();
    const reportedIp = screen.getByText("8.8.8.8 · unverified");
    expect(reportedIp).not.toHaveClass("text-warning");
    expect(reportedIp).not.toHaveClass("text-destructive");
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

    expect(screen.getByText(/^32 seconds ago · /)).toBeInTheDocument();
    const nearExpiry = screen.getByText("0:02");
    expect(nearExpiry).toHaveClass("text-warning");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    expect(screen.getByText("Expired")).toHaveClass("text-destructive");
    expect(
      screen.queryByRole("button", { name: "Approve" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reject" }),
    ).not.toBeInTheDocument();
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
      expect(
        screen.queryByRole("button", { name: "Approve" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Reject" }),
      ).not.toBeInTheDocument();
    },
  );
});
