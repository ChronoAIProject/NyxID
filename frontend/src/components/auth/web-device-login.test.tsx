import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WebDeviceLogin } from "./web-device-login";

const { deviceState, mockStart, mockGenerateNew, mockClose, mockNavigate } =
  vi.hoisted(() => ({
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
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,qr") },
}));

beforeEach(() => {
  vi.clearAllMocks();
  deviceState.phase = "idle";
  deviceState.request = null;
  deviceState.remainingSeconds = null;
  deviceState.error = null;
});

describe("WebDeviceLogin", () => {
  it("does not start a request on mount, then starts on explicit click", async () => {
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    expect(mockStart).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Sign in with the NyxID app" }));
    expect(mockStart).toHaveBeenCalledTimes(1);
  });

  it("shows the QR code, formatted user code, and manual-entry URI", async () => {
    deviceState.phase = "pending";
    deviceState.request = {
      device_code: "nyx_adc_test",
      user_code: "ABCDEFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete:
        "https://id.example/login/device?user_code=ABCD-EFGH",
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
    expect(screen.getByText("Expires in 597s")).toBeInTheDocument();
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
