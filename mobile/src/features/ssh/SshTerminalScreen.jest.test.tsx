import React from "react";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import { AppState, type AppStateStatus } from "react-native";
import { SshTerminalScreen } from "./SshTerminalScreen";
import { servicesApi } from "../../lib/api/servicesApi";
import { refreshAccessTokenIfNeeded } from "../../lib/api/http";
import * as Clipboard from "expo-clipboard";
const mockInject = jest.fn();
let mockFocused = true;
const mockNavigation = {
  setOptions: jest.fn(),
  navigate: jest.fn(),
  push: jest.fn(),
};
const mockAppHandlers: ((value: AppStateStatus) => void)[] = [];
jest.mock("@react-navigation/native", () => ({
  useIsFocused: () => mockFocused,
  useNavigation: () => mockNavigation,
}));
jest.mock("../../theme/ThemeContext", () => ({
  useTheme: () => ({
    colors: { bg: "#000000", textPrimary: "#ffffff", primary: "#8b5cf6" },
  }),
}));
jest.mock("../../i18n/LanguageProvider", () => ({
  useAppLanguage: () => ({ t: (value: string) => value }),
}));
jest.mock("../../components/workspace/WorkspaceUI", () => ({
  WorkspacePage: ({ children }: any) => children,
  Notice: ({ message }: any) => {
    const { Text } = require("react-native");
    return <Text>{message}</Text>;
  },
}));
jest.mock("../../components/settings/SettingsList", () => ({
  SettingsGroup: ({ children, footer }: any) => {
    const { Text } = require("react-native");
    return (
      <>
        {children}
        {footer ? <Text>{footer}</Text> : null}
      </>
    );
  },
  SettingsRow: ({ label, onPress }: any) => {
    const { Text } = require("react-native");
    return <Text onPress={onPress}>{label}</Text>;
  },
}));
jest.mock("react-native-webview", () => {
  const React = require("react");
  return {
    WebView: React.forwardRef((_props: any, ref: any) => {
      const { View } = require("react-native");
      React.useImperativeHandle(ref, () => ({
        injectJavaScript: mockInject,
        reload: jest.fn(),
      }));
      return <View testID="terminal-webview" {..._props} />;
    }),
  };
});
jest.mock("../../lib/api/baseUrl", () => ({
  getApiBaseUrl: () => "https://active-api.example/api/v1",
}));
jest.mock("../../lib/api/http", () => ({
  clientHeaders: () => ({
    "X-NyxID-Client": "mobile",
    "X-NyxID-Client-Version": "1.0.4",
  }),
  refreshAccessTokenIfNeeded: jest.fn(async () => true),
}));
jest.mock("../../lib/auth/sessionStore", () => ({
  loadStoredAuthSession: async () => ({ accessToken: "SENTINEL_BEARER_TOKEN" }),
}));
jest.mock("../../lib/auth/authAccount", () => ({
  getAuthAccountId: () => "account",
  subscribeAuthAccount: () => () => {},
}));
jest.mock("../../lib/env", () => ({
  getFrontendUrl: () => "https://web.example",
}));
jest.mock("../../lib/api/servicesApi", () => ({
  servicesApi: { connection: jest.fn(), connections: jest.fn() },
}));
jest.mock("expo-clipboard", () => ({
  getStringAsync: jest.fn(async () => "ls\n"),
}));
jest.mock("expo-web-browser", () => ({ openBrowserAsync: jest.fn() }));
class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 1;
  binaryType = "";
  onmessage: ((event: any) => void) | null = null;
  onerror: ((event: any) => void) | null = null;
  onclose: ((event?: any) => void) | null = null;
  onopen = null;
  send = jest.fn();
  close = jest.fn();
  constructor(
    public url: string,
    _protocols: unknown,
    public options: unknown,
  ) {
    Socket.instances.push(this);
  }
}
const connection = {
  id: "connection",
  slug: "ssh",
  label: "SSH",
  status: "active",
  is_active: true,
  auto_connected: false,
  credential_type: "none",
  service_type: "ssh",
  ssh_auth_mode: "cert" as const,
  ssh_allowed_principals: ["ops"],
  catalog_service_id: "catalog",
};
beforeEach(() => {
  AppState.currentState = "active";
  mockFocused = true;
  Socket.instances = [];
  mockInject.mockClear();
  global.WebSocket = Socket as unknown as typeof WebSocket;
  jest.mocked(servicesApi.connection).mockResolvedValue(connection);
  jest.mocked(servicesApi.connections).mockResolvedValue([connection]);
  jest
    .spyOn(AppState, "addEventListener")
    .mockImplementation((_event, handler) => {
      mockAppHandlers.push(handler);
      return { remove: jest.fn() };
    });
});
afterEach(() => {
  jest.restoreAllMocks();
  mockAppHandlers.length = 0;
});
const message = async (view: any, data: unknown) => {
  await act(async () => {
    view.props.onMessage({ nativeEvent: { data: JSON.stringify(data) } });
  });
};
const renderTerminal = async (element: React.ReactElement) => {
  const route = (element.props as any).route;
  if (route?.params && route.params.autoConnect === undefined) {
    return render(
      React.cloneElement(element, {
        route: {
          ...route,
          params: { ...route.params, autoConnect: true },
        },
      } as any),
    );
  }
  return render(element);
};
const waitForSocket = () =>
  waitFor(() => expect(Socket.instances.length).toBeGreaterThan(0));

test("bearer stays in the RN socket header; inline HTML, bridge and every injected script stay token-free", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  expect(view.props.source.html).not.toContain("SENTINEL_BEARER_TOKEN");
  expect(view.props.source.baseUrl).toBe("about:blank");
  expect(view.props.originWhitelist).toEqual(["about:blank"]);
  expect(view.props.allowFileAccess).toBe(false);
  expect(view.props.hideKeyboardAccessoryView).toBe(true);
  expect(view.props.keyboardDisplayRequiresUserAction).toBe(true);
  await message(view, { type: "ready" });
  await waitForSocket();
  const socket = Socket.instances[0]!;
  expect(socket.options).toEqual({
    headers: {
      Authorization: "Bearer SENTINEL_BEARER_TOKEN",
      "X-NyxID-Client": "mobile",
      "X-NyxID-Client-Version": "1.0.4",
    },
  });
  expect(socket.url).toContain("/ssh/catalog/terminal?");
  expect(socket.url).not.toContain("SENTINEL");
  await message(view, { type: "input", data: "ls\r" });
  expect(socket.send).toHaveBeenCalled();
  expect(typeof socket.send.mock.calls[0]![0]).not.toBe("string");
  await fireEvent.press(screen.getByLabelText("Up arrow"));
  expect(
    mockInject.mock.calls.every(
      ([script]) => !script.includes("SENTINEL_BEARER_TOKEN"),
    ),
  ).toBe(true);
  expect(
    view.props.onShouldStartLoadWithRequest({
      url: "about:blank",
      navigationType: "other",
    }),
  ).toBe(true);
  expect(
    view.props.onShouldStartLoadWithRequest({ url: "https://evil.example" }),
  ).toBe(false);
  expect(view.props.onShouldStartLoadWithRequest({ url: "about:blank" })).toBe(
    false,
  );
  await screen.unmount();
  expect(socket.close).toHaveBeenCalledWith(1000);
});

test("background grace keeps the socket for 15 seconds while inactive stays harmless; 401 retries once", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }} />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await act(async () => {
    Socket.instances[0]!.onerror?.({});
    expect(Socket.instances.length).toBe(1);
    Socket.instances[0]!.onclose?.({
      code: 1006,
      reason: "Expected HTTP 101 response but was '401 Unauthorized'",
    });
  });
  expect(refreshAccessTokenIfNeeded).toHaveBeenCalledWith(true);
  expect(Socket.instances.length).toBe(2);
  await act(async () => {
    Socket.instances[1]!.onerror?.({ status: 401 });
  });
  expect(Socket.instances.length).toBe(2);
  await fireEvent.press(screen.getByText("Reconnect"));
  expect(Socket.instances.length).toBe(3);
  await act(async () => {
    mockAppHandlers.forEach((handler) => handler("inactive"));
  });
  expect(Socket.instances[2]!.close).not.toHaveBeenCalled();
  await act(async () => {
    mockAppHandlers.forEach((handler) => handler("background"));
  });
  expect(Socket.instances[2]!.close).not.toHaveBeenCalled();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(15_000);
  });
  expect(Socket.instances[2]!.close).toHaveBeenCalledWith(1000);
  expect(screen.getByText("Disconnected")).toBeTruthy();
  await act(async () => {
    mockAppHandlers.forEach((handler) => handler("active"));
  });
  expect(Socket.instances.length).toBe(3);
  mockFocused = false;
  await screen.rerender(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }} />,
  );
  await screen.unmount();
  jest.useRealTimers();
});

test("background grace restarts after an active return and unmount clears it", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  const socket = Socket.instances[0]!;
  await act(async () => {
    await jest.advanceTimersByTimeAsync(2000);
  });
  expect(screen.getByText("Waiting for approval or the node…")).toBeTruthy();
  mockAppHandlers.forEach((handler) => handler("background"));
  await act(async () => {
    await jest.advanceTimersByTimeAsync(10_000);
  });
  mockAppHandlers.forEach((handler) => handler("active"));
  mockAppHandlers.forEach((handler) => handler("background"));
  await act(async () => {
    await jest.advanceTimersByTimeAsync(14_999);
  });
  expect(socket.close).not.toHaveBeenCalled();
  await screen.unmount();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(1);
  });
  expect(socket.close).toHaveBeenCalledWith(1000);
  expect(socket.close).toHaveBeenCalledTimes(1);
  jest.useRealTimers();
});

test.each([
  [14_999, false],
  [15_000, true],
])("wall-clock background grace at %s ms is enforced when timers do not run", async (elapsed, closes) => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  const socket = Socket.instances[0]!;
  const startedAt = Date.now();
  await act(async () => {
    mockAppHandlers.forEach((handler) => handler("background"));
    jest.setSystemTime(startedAt + elapsed);
    mockAppHandlers.forEach((handler) => handler("active"));
  });
  expect(socket.close).toHaveBeenCalledTimes(closes ? 1 : 0);
  if (!closes) expect(Socket.instances).toHaveLength(1);
  await screen.unmount();
  jest.useRealTimers();
});

test("a 403 after the upgrade entered approval wait explains denial or expiry", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(2000);
  });
  expect(screen.getByText("Waiting for approval or the node…")).toBeTruthy();
  await act(async () => {
    Socket.instances[0]!.onerror?.({});
    Socket.instances[0]!.onclose?.({
      reason: "Unexpected server response: 403",
    });
  });
  expect(
    screen.getByText(
      "The server refused this session. An approval may have been denied or expired, or you may no longer have access. Reconnect to try again.",
    ),
  ).toBeTruthy();
  expect(screen.queryByText("You don't have permission to open this terminal.")).toBeNull();
  await screen.unmount();
  jest.useRealTimers();
});

test("an immediate 403 retains the permission guidance", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await act(async () => {
    Socket.instances[0]!.onerror?.({ status: 403 });
  });
  expect(screen.getByText("You don't have permission to open this terminal.")).toBeTruthy();
  expect(
    screen.queryByText(
      "The server refused this session. An approval may have been denied or expired, or you may no longer have access. Reconnect to try again.",
    ),
  ).toBeNull();
  await screen.unmount();
});

test("approval review returns to the same waiting terminal and an approval unblock continues it", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  const view = screen.getByTestId("terminal-webview");
  await message(view, { type: "ready" });
  await waitForSocket();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(2000);
  });
  await fireEvent.press(screen.getByText("Review approvals"));
  expect(mockNavigation.push).toHaveBeenCalledWith("ActivityReview", undefined);
  const socket = Socket.instances[0]!;
  await act(async () => {
    socket.onmessage?.({
      data: JSON.stringify({ type: "connected", cols: 80, rows: 24 }),
    });
  });
  expect(Socket.instances).toHaveLength(1);
  expect(screen.getByText("Connected")).toBeTruthy();
  expect(
    mockInject.mock.calls.some(([script]) => script.includes('"type":"reset"')),
  ).toBe(true);
  await screen.unmount();
  jest.useRealTimers();
});

test.each([
  "Approval was denied by the account owner.",
  "This approval expired before the terminal opened.",
])(
  "approval closure keeps the server message and reconnect action: %s",
  async (detail) => {
    const screen = await renderTerminal(
      <SshTerminalScreen
        route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
      />,
    );
    const view = screen.getByTestId("terminal-webview");
    await message(view, { type: "ready" });
    await waitForSocket();
    const socket = Socket.instances[0]!;
    await act(async () => {
      socket.onmessage?.({
        data: JSON.stringify({ type: "error", message: detail }),
      });
    });
    expect(screen.getByText(detail)).toBeTruthy();
    expect(screen.getByText("Reconnect")).toBeTruthy();
    await screen.unmount();
  },
);

test("a pending approval gets a review action without closing on route blur", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(2000);
  });
  expect(screen.getByText("Waiting for approval or the node…")).toBeTruthy();
  await fireEvent.press(screen.getByText("Review approvals"));
  expect(mockNavigation.push).toHaveBeenCalledWith("ActivityReview", undefined);
  mockFocused = false;
  await screen.rerender(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }}
    />,
  );
  expect(Socket.instances[0]!.close).not.toHaveBeenCalled();
  await screen.unmount();
  jest.useRealTimers();
});

test("sticky Ctrl-C sends a binary interrupt once and releases the selected key", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  await message(view, { type: "ready" });
  await waitForSocket();
  await fireEvent.press(screen.getByLabelText("Control"));
  expect(screen.getByLabelText("Control (armed)").props.accessibilityState.selected).toBe(true);
  await message(view, { type: "input", data: "c" });
  expect([...Socket.instances[0]!.send.mock.calls[0]![0]]).toEqual([3]);
  expect(screen.getByLabelText("Control").props.accessibilityState.selected).toBe(false);
  await message(view, { type: "input", data: "c" });
  expect([...Socket.instances[0]!.send.mock.calls[1]![0]]).toEqual([99]);
  await screen.unmount();
});

test("a terminated renderer gets a fresh inline document and navigation allowance", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection" } }} />,
  );
  const first = screen.getByTestId("terminal-webview");
  first.props.onLoadStart();
  await message(first, { type: "ready" });
  await waitForSocket();
  expect(Socket.instances.length).toBe(1);
  await act(async () => {
    first.props.onContentProcessDidTerminate();
  });
  expect(Socket.instances[0]!.close).toHaveBeenCalledWith(1000);
  expect(
    screen.getByText("The terminal display stopped. Connect again to continue."),
  ).toBeTruthy();
  expect(screen.queryByTestId("terminal-webview")).toBeNull();
  await fireEvent.press(screen.getByLabelText("Connect"));
  const replacement = screen.getByTestId("terminal-webview");
  expect(replacement.props.onShouldStartLoadWithRequest({ url: "about:blank", navigationType: "other" })).toBe(true);
  expect(replacement.props.onShouldStartLoadWithRequest({ url: "about:blank" })).toBe(false);
  expect(Socket.instances.length).toBe(1);
  await message(replacement, { type: "ready" });
  expect(Socket.instances.length).toBe(2);
  await screen.unmount();
});

test.each(["The credential node is offline", "SSH host key mismatch"])(
  "server errors retain actionable node/host messages: %s",
  async (detail) => {
    const screen = await renderTerminal(
      <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection" } }} />,
    );
    await message(screen.getByTestId("terminal-webview"), { type: "ready" });
    await waitForSocket();
    const socket = Socket.instances[0]!;
    await act(async () => {
      socket.onmessage?.({
        data: JSON.stringify({ type: "connected", cols: 80, rows: 24 }),
      });
    });
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(
      mockInject.mock.calls.some(([script]) => script.includes('"type":"reset"')),
    ).toBe(true);
    await act(async () => {
      socket.onmessage?.({
        data: JSON.stringify({ type: "error", message: detail }),
      });
      socket.onerror?.({ status: 503 });
      socket.onclose?.({ reason: "Unexpected server response: 503" });
    });
    expect(screen.getByText(detail)).toBeTruthy();
    expect(
      mockInject.mock.calls.some(
        ([script]) =>
          script.includes('"type":"session-ended"') &&
          script.includes(detail),
      ),
    ).toBe(true);
    expect(
      screen.queryByText("Couldn't open the terminal connection. Check your network and reconnect."),
    ).toBeNull();
    expect(screen.getByText("Reconnect")).toBeTruthy();
    expect(socket.close).toHaveBeenCalledWith(1000);
    await screen.unmount();
  },
);

test("remote close and invalid bridge data never replay shell input; resizing is debounced", async () => {
  jest.useFakeTimers();
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection" } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  await message(view, { type: "ready" });
  await waitForSocket();
  const socket = Socket.instances[0]!;
  await message(view, { type: "input", data: "bad", url: "https://evil.example" });
  await message(view, { type: "resize", cols: 1, rows: 2 });
  expect(socket.send).not.toHaveBeenCalled();
  await message(view, { type: "resize", cols: 80, rows: 24 });
  await message(view, { type: "resize", cols: 120, rows: 40 });
  await act(async () => {
    await jest.advanceTimersByTimeAsync(99);
  });
  expect(socket.send).not.toHaveBeenCalled();
  await act(async () => {
    await jest.advanceTimersByTimeAsync(1);
  });
  expect(socket.send).toHaveBeenCalledTimes(1);
  expect(socket.send).toHaveBeenLastCalledWith(
    JSON.stringify({ type: "resize", cols: 120, rows: 40 }),
  );
  await act(async () => {
    socket.onmessage?.({ data: JSON.stringify({ type: "closed" }) });
    socket.onclose?.();
  });
  expect(screen.getByText("Closed by remote host")).toBeTruthy();
  expect(
    mockInject.mock.calls.some(
      ([script]) =>
        script.includes('"type":"session-ended"') &&
        script.includes("Session closed by remote host."),
    ),
  ).toBe(true);
  await screen.unmount();
  jest.useRealTimers();
});

test("long server errors wrap in a full error line below the status row", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  const socket = Socket.instances[0]!;
  const detail =
    "No node agent is bound to this SSH service. Deploy a NyxID node agent on the target's network and verify its host key before reconnecting.";
  await act(async () => {
    socket.onmessage?.({
      data: JSON.stringify({ type: "error", message: detail }),
    });
  });
  const error = screen.getByText(detail);
  expect(error.props.numberOfLines).toBeUndefined();
  expect(error.props.accessibilityRole).toBe("alert");
  expect(screen.getByText("Disconnected")).toBeTruthy();
  await screen.unmount();
});

test("several principals require selection; a clipboard read resolving after blur cannot inject", async () => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "deploy"],
  });
  const screen = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection" } }} />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  expect(Socket.instances.length).toBe(0);
  expect(screen.getByText("Choose an SSH principal to connect.")).toBeTruthy();
  await fireEvent.press(screen.getByText("deploy"));
  expect(new URL(Socket.instances[0]!.url).searchParams.get("principal")).toBe(
    "deploy",
  );
  let resolve!: (value: string) => void;
  jest.mocked(Clipboard.getStringAsync).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  await fireEvent.press(screen.getByLabelText("Paste"));
  mockFocused = false;
  await screen.rerender(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: true } }} />,
  );
  const injections = mockInject.mock.calls.length;
  await act(async () => {
    resolve("private clipboard");
  });
  expect(mockInject).toHaveBeenCalledTimes(injections);
  expect(Socket.instances[0]!.close).not.toHaveBeenCalled();
  await screen.unmount();
});

test("pre-connect copy separates principal choice and keeps a persistent principal row", async () => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "deploy"],
  });
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", autoConnect: false } }}
    />,
  );
  expect(
    screen.getByText("Choose an SSH principal, then connect to {{connection}}."),
  ).toBeTruthy();
  expect(screen.getByText("SSH principal")).toBeTruthy();
  expect(screen.getByText("Connect")).toBeTruthy();
  await fireEvent.press(screen.getByText("deploy"));
  expect(screen.getByText("Connect to {{connection}} as {{principal}}?")).toBeTruthy();
  expect(screen.getByText("SSH principal")).toBeTruthy();
  await screen.unmount();
});

test("reconnect preserves a valid principal and clears an invalid one", async () => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "deploy"],
  });
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", principal: "ops", autoConnect: true } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  expect(screen.queryByText("deploy")).toBeNull();
  await message(screen.getByTestId("terminal-webview"), { type: "connected" });
  await act(async () => {
    Socket.instances[0]!.onclose?.({ code: 1000 });
  });
  await fireEvent.press(screen.getByText("Reconnect"));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(screen.queryByText("deploy")).toBeNull();
  await screen.unmount();

  Socket.instances = [];
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "deploy"],
  });
  const invalid = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection", principal: "ops", autoConnect: true } }}
    />,
  );
  await message(invalid.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await message(invalid.getByTestId("terminal-webview"), { type: "connected" });
  await act(async () => {
    Socket.instances[0]!.onclose?.({ code: 1000 });
  });
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["deploy", "dev"],
  });
  await fireEvent.press(invalid.getByText("Reconnect"));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(invalid.getByText("deploy")).toBeTruthy();
  await invalid.unmount();
});

test("a catalog deep link resolves the active connection, while an unknown catalog is actionable", async () => {
  const deep = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "catalog", autoConnect: false } }} />,
  );
  expect(deep.getByText("Connect")).toBeTruthy();
  expect(Socket.instances.length).toBe(0);
  await fireEvent.press(deep.getByText("Connect"));
  await message(deep.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  expect(Socket.instances[0]!.url).toContain("/ssh/catalog/terminal?");
  await deep.unmount();

  Socket.instances = [];
  jest.mocked(servicesApi.connections).mockResolvedValue([]);
  const unknown = await renderTerminal(
    <SshTerminalScreen route={{ params: { serviceId: "missing-catalog", autoConnect: false } }} />,
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(
    unknown.getByText(
      "This SSH service is not connected to your account. Open Services to connect it first.",
    ),
  ).toBeTruthy();
  await fireEvent.press(unknown.getByText("Open Services"));
  expect(mockNavigation.navigate).toHaveBeenCalledWith("Services");
  await unknown.unmount();
});

test("a link string autoConnect value never auto-connects the terminal", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{
        params: {
          serviceId: "catalog",
          connectionId: "connection",
          autoConnect: "true" as unknown as boolean,
        },
      }}
    />,
  );
  expect(screen.getByText("Connect")).toBeTruthy();
  expect(Socket.instances.length).toBe(0);
  await screen.unmount();
});

test("a connection load failure has an unavailable state without terminal chrome", async () => {
  jest.mocked(servicesApi.connection).mockRejectedValueOnce(new Error("offline"));
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(screen.getByText("This SSH connection could not be loaded.")).toBeTruthy();
  expect(screen.queryByText("Connecting…")).toBeNull();
  expect(screen.queryByTestId("terminal-webview")).toBeNull();
  await screen.unmount();
});

test("a disallowed deep-link principal shows the picker instead of hanging", async () => {
  jest.mocked(servicesApi.connections).mockResolvedValue([
    { ...connection, ssh_allowed_principals: ["ops", "dev"] },
  ]);
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", principal: "forbidden" } }}
    />,
  );
  expect(
    screen.getByText(
      "That SSH principal is not allowed for this service. Choose one below.",
    ),
  ).toBeTruthy();
  expect(screen.getByText("ops")).toBeTruthy();
  expect(Socket.instances.length).toBe(0);
  await screen.unmount();
});

test.each([
  [
    "node_key",
    "Certificate-mode services are required for the mobile terminal. Run nyxid ssh exec <service> from a computer.",
  ],
  [
    "proxy_only",
    "Certificate-mode services are required for the mobile terminal. Run nyxid ssh proxy from a computer.",
  ],
  ["cert", "Enable this connection to open the terminal."],
])("non-terminal SSH mode copy is actionable: %s", async (mode, copy) => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_auth_mode: mode,
    ...(mode === "cert" ? { is_active: false } : {}),
  });
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(screen.getByText(copy)).toBeTruthy();
  expect(screen.queryByText("Open web terminal")).toBeNull();
  await screen.unmount();
});

test("a route principal is applied only when changed; reconnect keeps the user's selection", async () => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "dev", "root"],
  });
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{
        params: {
          serviceId: "catalog",
          connectionId: "connection",
          principal: "ops",
          autoConnect: false,
        },
      }}
    />,
  );
  await fireEvent.press(screen.getByText("SSH principal"));
  await fireEvent.press(screen.getByText("dev"));
  await fireEvent.press(screen.getByLabelText("Connect"));
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  expect(
    new URL(Socket.instances.at(-1)!.url).searchParams.get("principal"),
  ).toBe("dev");
  await act(async () => Socket.instances.at(-1)!.onclose?.({}));
  await fireEvent.press(screen.getByText("Reconnect"));
  await waitFor(() => expect(Socket.instances.length).toBeGreaterThan(1));
  expect(
    new URL(Socket.instances.at(-1)!.url).searchParams.get("principal"),
  ).toBe("dev");
  await screen.rerender(
    <SshTerminalScreen
      route={{
        params: {
          serviceId: "catalog",
          connectionId: "connection",
          principal: "root",
          autoConnect: false,
        },
      }}
    />,
  );
  await waitFor(() =>
    expect(
      new URL(Socket.instances.at(-1)!.url).searchParams.get("principal"),
    ).toBe("root"),
  );
  await screen.unmount();
});

test.each([
  [["ops", "root"], "ops"],
  [["root"], "root"],
] as const)(
  "a removed selection falls back through route then single principal: %s",
  async (allowed, expected) => {
    jest.mocked(servicesApi.connection).mockResolvedValue({
      ...connection, ssh_allowed_principals: ["ops", "dev"],
    });
    const screen = await renderTerminal(
      <SshTerminalScreen
        route={{
          params: {
            serviceId: "catalog",
            connectionId: "connection",
            principal: "ops",
            autoConnect: false,
          },
        }}
      />,
    );
    await fireEvent.press(screen.getByText("SSH principal"));
    await fireEvent.press(screen.getByText("dev"));
    await fireEvent.press(screen.getByLabelText("Connect"));
    await message(screen.getByTestId("terminal-webview"), { type: "ready" });
    await waitForSocket();
    await act(async () => Socket.instances.at(-1)!.onclose?.({}));
    jest.mocked(servicesApi.connection).mockResolvedValue({
      ...connection, ssh_allowed_principals: [...allowed],
    });
    await fireEvent.press(screen.getByText("Reconnect"));
    await waitFor(() =>
      expect(
        new URL(Socket.instances.at(-1)!.url).searchParams.get("principal"),
      ).toBe(expected),
    );
    await screen.unmount();
  },
);

test("renderer death in background requires an explicit Connect after returning", async () => {
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  const first = screen.getByTestId("terminal-webview");
  await message(first, { type: "ready" });
  await waitForSocket();
  await act(async () => {
    AppState.currentState = "background";
    mockAppHandlers.forEach((handler) => handler("background"));
    first.props.onRenderProcessGone();
  });
  await message(first, { type: "ready" });
  expect(Socket.instances).toHaveLength(1);
  await act(async () => {
    AppState.currentState = "active";
    mockAppHandlers.forEach((handler) => handler("active"));
  });
  expect(Socket.instances).toHaveLength(1);
  await fireEvent.press(screen.getByLabelText("Connect"));
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitFor(() => expect(Socket.instances).toHaveLength(2));
  await screen.unmount();
});

test.each([14999, 15000])(
  "an attempt starting backgrounded defers the socket and enforces wall clock: %s",
  async (elapsed) => {
    jest.useFakeTimers();
    jest.setSystemTime(100000);
    AppState.currentState = "background";
    const screen = await renderTerminal(
      <SshTerminalScreen
        route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
      />,
    );
    await message(screen.getByTestId("terminal-webview"), { type: "ready" });
    expect(Socket.instances).toHaveLength(0);
    jest.setSystemTime(100000 + elapsed);
    await act(async () => {
      AppState.currentState = "active";
      mockAppHandlers.forEach((handler) => handler("active"));
    });
    expect(Socket.instances).toHaveLength(elapsed < 15000 ? 1 : 0);
    if (elapsed >= 15000) expect(screen.getByText("Disconnected")).toBeTruthy();
    await screen.unmount();
    jest.useRealTimers();
  },
);

test("truncated paste is an informational notice without error or reconnect on a healthy session", async () => {
  jest.mocked(Clipboard.getStringAsync).mockResolvedValueOnce("a".repeat(65537));
  const screen = await renderTerminal(
    <SshTerminalScreen
      route={{ params: { serviceId: "catalog", connectionId: "connection" } }}
    />,
  );
  await message(screen.getByTestId("terminal-webview"), { type: "ready" });
  await waitForSocket();
  await act(async () =>
    Socket.instances[0]!.onmessage?.({
      data: JSON.stringify({ type: "connected" }),
    }),
  );
  await fireEvent.press(screen.getByLabelText("Paste"));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(
    screen.getByText("Only the first 65,536 characters were pasted."),
  ).toBeTruthy();
  expect(screen.queryByText("Reconnect")).toBeNull();
  expect(screen.getByText("Connected")).toBeTruthy();
  await fireEvent.press(screen.getByLabelText("Paste"));
  expect(
    screen.queryByText("Only the first 65,536 characters were pasted."),
  ).toBeNull();
  await screen.unmount();
});
