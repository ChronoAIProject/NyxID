import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { formatWebAuthDeviceRemaining } from "@/lib/auth-device-time";
import { WebDeviceLogin } from "./web-device-login";

const {
  deviceState,
  mockStart,
  mockGenerateNew,
  mockClose,
  mockNavigate,
  mockQrToDataURL,
} = vi.hoisted(() => ({
    deviceState: {
      phase: "idle" as string,
      request: null as Record<string, unknown> | null,
      remainingSeconds: null as number | null,
      error: null as { code: number | null; message: string } | null,
    },
    mockStart: vi.fn(),
    mockGenerateNew: vi.fn(),
    mockClose: vi.fn(),
    mockNavigate: vi.fn(),
    // Arg shape is asserted explicitly in the QR test below, so the mock
    // itself stays untyped — named-but-unused params trip no-unused-vars.
    mockQrToDataURL: vi.fn(() => Promise.resolve("data:image/png;base64,qr")),
  }));

vi.mock("@/hooks/use-auth-device", () => ({
  useWebAuthDeviceLogin: () => ({
    ...deviceState,
    start: mockStart,
    generateNew: mockGenerateNew,
    close: mockClose,
  }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("qrcode", () => ({
  default: { toDataURL: mockQrToDataURL },
}));

function relativeLuminance(color: string): number {
  const linearChannel = (start: number) => {
    const channel = Number.parseInt(color.slice(start, start + 2), 16) / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  };

  return (
    0.2126 * linearChannel(1) +
    0.7152 * linearChannel(3) +
    0.0722 * linearChannel(5)
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  deviceState.phase = "idle";
  deviceState.request = null;
  deviceState.remainingSeconds = null;
  deviceState.error = null;
});

describe("WebDeviceLogin", () => {
  it("formats the countdown as mm:ss", () => {
    expect(formatWebAuthDeviceRemaining(583)).toBe("9:43");
    expect(formatWebAuthDeviceRemaining(0)).toBe("0:00");
    expect(formatWebAuthDeviceRemaining(-1)).toBe("0:00");
  });

  it("does not start a request on mount, then starts on explicit click", async () => {
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    expect(mockStart).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Sign in with the NyxID app" }));
    expect(mockStart).toHaveBeenCalledTimes(1);
  });

  it("shows the QR code, formatted user code, and manual-entry URI", async () => {
    const qrPayload =
      "https://id.example/login/device?user_code=ABCD-EFGH";
    deviceState.phase = "pending";
    deviceState.request = {
      device_code: "nyx_adc_test",
      user_code: "ABCDEFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete: qrPayload,
      expires_in: 600,
      interval: 5,
    };
    deviceState.remainingSeconds = 597;
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    await user.click(screen.getByRole("button", { name: "Sign in with the NyxID app" }));

    expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    expect(screen.getByText("https://id.example/login/device")).toBeInTheDocument();
    expect(screen.getByAltText("Scan this QR code with the NyxID app")).toBeInTheDocument();
    expect(screen.getByText("Expires in 9:57")).toBeInTheDocument();

    const expectedOptions = {
      errorCorrectionLevel: "M",
      margin: 4,
      width: 208,
      color: { dark: "#0c0b14", light: "#e8e4f0" },
    };
    await waitFor(() => {
      expect(mockQrToDataURL.mock.calls).toEqual([
        [qrPayload, expectedOptions],
      ]);
    });

    expect(relativeLuminance(expectedOptions.color.dark)).toBeLessThan(
      relativeLuminance(expectedOptions.color.light),
    );
  });

  it("renders a denied terminal state with an explicit regenerate action", async () => {
    deviceState.phase = "denied";
    deviceState.request = {
      device_code: "nyx_adc_test",
      user_code: "ABCD-EFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete:
        "https://id.example/login/device?user_code=ABCD-EFGH",
      expires_in: 600,
      interval: 5,
    };
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    await user.click(screen.getByRole("button", { name: "Sign in with the NyxID app" }));

    expect(screen.getByText("Sign-in was rejected")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Generate new code" }));
    expect(mockGenerateNew).toHaveBeenCalledTimes(1);
  });

  it("routes through the normal dashboard destination after cookie adoption", async () => {
    deviceState.phase = "success";
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    await user.click(screen.getByRole("button", { name: "Sign in with the NyxID app" }));

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
  });
});
