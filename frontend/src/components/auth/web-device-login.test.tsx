import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { requestResponseSchema } from "@/schemas/auth-device";
import { appI18n } from "@/i18n";
import { formatWebAuthDeviceRemaining } from "@/lib/auth-device-time";
import { WebDeviceLogin } from "./web-device-login";

const {
  deviceState,
  mockStart,
  mockGenerateNew,
  mockClose,
  mockNavigate,
  mockQrToDataURL,
  mockIsRequestActive,
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
    mockIsRequestActive: vi.fn(() => true),
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
    isRequestActive: mockIsRequestActive,
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

let coarsePointer = false;
beforeEach(() => {
  vi.clearAllMocks();
  mockIsRequestActive.mockReturnValue(true);
  coarsePointer = false;
  const media = window.matchMedia("(pointer: coarse)");
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    ...media,
    media: query,
    matches: query === "(pointer: coarse)" && coarsePointer,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
  deviceState.phase = "idle";
  deviceState.request = null;
  deviceState.remainingSeconds = null;
  deviceState.error = null;
});

afterEach(async () => {
  vi.restoreAllMocks();
  await appI18n.changeLanguage("en");
});

describe("WebDeviceLogin", () => {
  it("translates an existing connection failure and its retry control when language changes", async () => {
    const user = userEvent.setup();
    const message = "Couldn't reach NyxID. Check your connection and try again.";
    deviceState.phase = "error";
    deviceState.error = { code: null, message };
    render(<WebDeviceLogin />);
    await user.click(screen.getByRole("button", { name: "Continue with the NyxID app" }));
    expect(screen.getByText(message)).toBeVisible();

    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByText("无法连接到 NyxID。请检查网络连接后重试。")).toBeVisible();
    expect(screen.getByRole("button", { name: "生成新验证码" })).toBeVisible();
    await act(async () => { await appI18n.changeLanguage("zh-TW"); });
    expect(screen.getByText("無法連線至 NyxID。請檢查網路連線後再試。")).toBeVisible();
    expect(screen.getByRole("button", { name: "產生新驗證碼" })).toBeVisible();
    expect(screen.queryByText(message)).not.toBeInTheDocument();
    expect(deviceState.error).toEqual({ code: null, message });
    expect(mockStart).toHaveBeenCalledTimes(1);
  });
  it("formats the countdown as mm:ss", () => {
    expect(formatWebAuthDeviceRemaining(583)).toBe("9:43");
    expect(formatWebAuthDeviceRemaining(0)).toBe("0:00");
    expect(formatWebAuthDeviceRemaining(-1)).toBe("0:00");
  });

  it("does not start a request on mount, then starts on explicit click", async () => {
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    expect(mockStart).not.toHaveBeenCalled();

    const trigger = screen.getByRole("button", {
      name: "Continue with the NyxID app",
    });
    expect(trigger).toHaveClass("min-h-[46px]", "border-border", "bg-background", "px-4");
    expect(trigger.querySelector("img")).toHaveAttribute(
      "src",
      "/nyxid-coloured-icon.svg",
    );
    await user.click(trigger);
    expect(mockStart).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("heading", { name: "Continue with the NyxID app" }),
    ).toHaveFocus();
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
    await user.click(
      screen.getByRole("button", { name: "Continue with the NyxID app" }),
    );

    expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    expect(screen.getByText("https://id.example/login/device")).toBeInTheDocument();
    expect(
      await screen.findByAltText("QR code to continue with the NyxID app"),
    ).toBeInTheDocument();
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

  it("tears down on back and restores focus to the trigger row", async () => {
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    await user.click(
      screen.getByRole("button", { name: "Continue with the NyxID app" }),
    );

    await user.click(
      screen.getByRole("button", { name: "Back to all sign-in options" }),
    );

    expect(mockClose).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Continue with the NyxID app" }),
      ).toHaveFocus(),
    );
  });

  const nativeRequest = {
    device_code: "nyx_adc_private_poll_secret",
    user_code: "ABCD-EFGH",
    verification_uri: "https://id.example/login/device",
    verification_uri_complete: "https://id.example/login/device?user_code=ABCD-EFGH",
    app_verification_uri: "https://api.example/login/device?user_code=ABCD-EFGH",
    expires_in: 60,
    interval: 5,
  };

  function pendingNativeRequest() {
    coarsePointer = true;
    deviceState.phase = "pending";
    deviceState.remainingSeconds = 60;
    deviceState.request = nativeRequest;
  }

  it("only the ready opted-in mobile QR opens a confirmation, without a direct QR href", async () => {
    pendingNativeRequest();
    const user = userEvent.setup();
    render(<WebDeviceLogin isOpen allowAppHandoff />);
    const qr = await screen.findByRole("button", { name: "Open NyxID app from QR code" });
    expect(qr).not.toHaveAttribute("href");
    expect(qr).not.toHaveAttribute("tabindex", "-1");
    expect(qr).toContainElement(screen.getByAltText("QR code to continue with the NyxID app"));
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(qr);
    expect(screen.getByRole("dialog", { name: "Open NyxID app?" })).toBeVisible();
    const confirm = screen.getByRole("link", { name: "Open NyxID app" });
    expect(confirm).toHaveClass("nyx-gradient-vivid");
    expect(confirm).toHaveAttribute("href", nativeRequest.app_verification_uri);
    expect(confirm).toHaveAttribute("target", "_blank");
    expect(confirm).toHaveAttribute("rel", "noopener noreferrer");
    expect(confirm).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(document.querySelector('[href*="nyx_adc_private"]')).toBeNull();
    expect(mockIsRequestActive).toHaveBeenLastCalledWith(nativeRequest);
    expect(mockClose).not.toHaveBeenCalled();
  });

  it.each(["Cancel", "Escape", "Close"])("%s dismisses the confirmation and returns focus to the QR", async (action) => {
    pendingNativeRequest();
    const user = userEvent.setup();
    render(<WebDeviceLogin isOpen allowAppHandoff />);
    const qr = await screen.findByRole("button", { name: "Open NyxID app from QR code" });
    qr.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("dialog")).toBeVisible();
    if (action === "Escape") await user.keyboard("{Escape}");
    else await user.click(screen.getByRole("button", { name: action }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    await waitFor(() => expect(qr).toHaveFocus());
  });

  it.each([
    ["en", "Open NyxID app?", "Open NyxID app", "Tap the QR code to open NyxID, or scan it with another device.", "Open this login in the app without scanning. Review and approve it, then return to this browser tab."],
    ["zh-CN", "打开 NyxID 应用？", "打开 NyxID 应用", "点击二维码打开 NyxID，或使用其他设备扫描。", "无需扫码，直接在应用中打开此次登录请求。请查看并批准，然后返回此浏览器标签页。"],
    ["zh-TW", "開啟 NyxID 應用程式？", "開啟 NyxID 應用程式", "點選 QR 碼開啟 NyxID，或使用其他裝置掃描。", "無需掃碼，直接在應用程式中開啟此次登入請求。請檢視並核准，然後返回此瀏覽器分頁。"],
  ])("localizes the QR hint and confirmation in %s", async (language, title, open, hint, instructions) => {
    pendingNativeRequest();
    await appI18n.changeLanguage(language);
    render(<WebDeviceLogin isOpen allowAppHandoff />);
    await waitFor(() => expect(screen.getByText(hint)).toBeVisible());
    fireEvent.click(screen.getByRole("img").closest("button")!);
    expect(screen.getByRole("dialog", { name: title })).toHaveAccessibleDescription(instructions);
    expect(screen.getByRole("link", { name: open })).toHaveClass("nyx-gradient-vivid");
  });

  it("uses web verification payloads and plain QR by default even with a native URL", async () => {
    pendingNativeRequest();
    render(<WebDeviceLogin isOpen />);
    const qr = await screen.findByAltText("QR code to continue with the NyxID app");
    expect(qr.closest("button")).toBeNull();
    expect(mockQrToDataURL).toHaveBeenCalledWith(nativeRequest.verification_uri_complete, expect.anything());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("preparation and stale images cannot open the dialog before the current image is ready", async () => {
    pendingNativeRequest();
    let finishFirst!: (value: string) => void;
    let finishNext!: (value: string) => void;
    mockQrToDataURL.mockImplementationOnce(() => new Promise<string>((resolve) => { finishFirst = resolve; }));
    const { rerender } = render(<WebDeviceLogin isOpen allowAppHandoff />);
    expect(screen.getByText("Preparing QR code...")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Open NyxID app from QR code" })).not.toBeInTheDocument();
    await act(async () => { finishFirst("data:image/png;base64,first"); });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Open NyxID app from QR code" }));
    mockQrToDataURL.mockImplementationOnce(() => new Promise<string>((resolve) => { finishNext = resolve; }));
    deviceState.request = { ...nativeRequest, user_code: "JKMN-PQRS", app_verification_uri: "https://api.example/login/device?user_code=JKMN-PQRS" };
    rerender(<WebDeviceLogin isOpen allowAppHandoff />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open NyxID app from QR code" })).not.toBeInTheDocument();
    await act(async () => { finishNext("data:image/png;base64,next"); });
    expect(screen.getByRole("button", { name: "Open NyxID app from QR code" })).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it.each(["expiry", "denied", "used", "error", "restoring", "replacement", "pointer", "prop", "closed"])("retires a confirmation on %s without reviving it later", async (change) => {
    pendingNativeRequest();
    const user = userEvent.setup();
    const { rerender } = render(<WebDeviceLogin isOpen allowAppHandoff />);
    await user.click(await screen.findByRole("button", { name: "Open NyxID app from QR code" }));
    if (change === "expiry") deviceState.remainingSeconds = 0;
    else if (change === "replacement") deviceState.request = { ...nativeRequest };
    else if (change === "pointer") coarsePointer = false;
    else if (change !== "prop" && change !== "closed") deviceState.phase = change;
    rerender(<WebDeviceLogin isOpen={change !== "closed"} allowAppHandoff={change !== "prop"} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    pendingNativeRequest();
    rerender(<WebDeviceLogin isOpen allowAppHandoff />);
    await screen.findByRole("button", { name: "Open NyxID app from QR code" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("keeps older backends usable and removes app links when the request expires", () => {
    const webLink = "https://id.example/login/device?user_code=ABCD-EFGH";
    deviceState.phase = "pending";
    deviceState.request = {
      user_code: "ABCD-EFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete: webLink,
      expires_in: 600,
      interval: 5,
    };
    const { rerender } = render(<WebDeviceLogin isOpen />);
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    deviceState.phase = "expired";
    rerender(<WebDeviceLogin isOpen />);
    expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  });

  it("hides expired QR/tap/manual-code controls while checking an existing delivery and translates its status", async () => {
    coarsePointer = true;
    deviceState.phase = "pending";
    deviceState.remainingSeconds = 0;
    // Also fence the UI if its request object is still present in this render.
    deviceState.request = {
      user_code: "ABCD-EFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete: "https://id.example/login/device?user_code=ABCD-EFGH",
      app_verification_uri: "https://api.example/login/device?user_code=ABCD-EFGH",
      expires_in: 60,
      interval: 5,
    };
    const { rerender } = render(<WebDeviceLogin isOpen />);
    expect(screen.getByRole("status")).toHaveTextContent("Checking your request…");
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.queryByAltText("QR code to continue with the NyxID app")).not.toBeInTheDocument();
    expect(screen.queryByText("ABCD-EFGH")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy sign-in code" })).not.toBeInTheDocument();
    deviceState.request = null;
    rerender(<WebDeviceLogin isOpen />);
    expect(screen.getByRole("status")).toHaveTextContent("Checking your request…");
    // A backward wall-clock step can make the countdown positive after the
    // hook permanently retired the public request. Keep the pending UI visible.
    deviceState.remainingSeconds = 2;
    rerender(<WebDeviceLogin isOpen />);
    expect(screen.getByRole("status")).toHaveTextContent("Checking your request…");
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.queryByAltText("QR code to continue with the NyxID app")).not.toBeInTheDocument();
    expect(screen.queryByText("ABCD-EFGH")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy sign-in code" })).not.toBeInTheDocument();
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByRole("status")).toHaveTextContent("正在检查你的请求…");
    await act(async () => { await appI18n.changeLanguage("zh-TW"); });
    expect(screen.getByRole("status")).toHaveTextContent("正在檢查你的請求…");
  });

  it.each([
    ["desktop fine pointer", false, "https://api.example/login/device?user_code=ABCD-EFGH"],
    ["older backend on touch", true, undefined],
    ["HTTP development on touch", true, "http://localhost:3001/login/device?user_code=ABCD-EFGH"],
    ["invalid optional link on touch", true, "javascript:alert(1)"],
  ])("keeps a plain QR with no navigation for %s", async (_label, coarse, appLink) => {
    coarsePointer = coarse;
    deviceState.phase = "pending";
    deviceState.request = requestResponseSchema.parse({
      device_code: "nyx_adc_private",
      user_code: "ABCD-EFGH",
      verification_uri: "https://id.example/login/device",
      verification_uri_complete: "https://id.example/login/device?user_code=ABCD-EFGH",
      app_verification_uri: appLink,
      expires_in: 600,
      interval: 5,
    });
    render(<WebDeviceLogin isOpen allowAppHandoff />);
    const qr = await screen.findByAltText("QR code to continue with the NyxID app");
    expect(qr.closest("button")).toBeNull();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(mockQrToDataURL).toHaveBeenCalledWith(
      deviceState.request.app_verification_uri ?? deviceState.request.verification_uri_complete,
      expect.anything(),
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
    await user.click(
      screen.getByRole("button", { name: "Continue with the NyxID app" }),
    );

    expect(screen.getByText("Sign-in was rejected")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Generate new code" }));
    expect(mockGenerateNew).toHaveBeenCalledTimes(1);
  });

  it("routes through the normal Assistant destination after cookie adoption", async () => {
    deviceState.phase = "success";
    const user = userEvent.setup();
    render(<WebDeviceLogin />);
    await user.click(
      screen.getByRole("button", { name: "Continue with the NyxID app" }),
    );

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/assistant" });
  });

  it.each([
    "/dashboard?explicit=1",
    "/oauth/authorize?client_id=app&state=review",
    "/orgs/join/invitation",
  ])("preserves the explicit returnTo %s after cookie adoption", async (path) => {
    deviceState.phase = "success";
    const returnTo = `${window.location.origin}${path}`;
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
    try {
      render(<WebDeviceLogin returnTo={returnTo} />);
      await waitFor(() => expect(assign).toHaveBeenCalledWith(returnTo));
      expect(mockNavigate).not.toHaveBeenCalled();
    } finally {
      assign.mockRestore();
    }
  });
});
