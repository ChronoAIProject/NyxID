import React from "react";
import { act, fireEvent, render } from "@testing-library/react-native";
import { AppState, type AppStateStatus } from "react-native";
import { SshTerminalScreen } from "./SshTerminalScreen";
import { servicesApi } from "../../lib/api/servicesApi";
import { refreshAccessTokenIfNeeded } from "../../lib/api/http";
import * as Clipboard from "expo-clipboard";
const mockInject = jest.fn();
let mockFocused = true;
const mockNavigation = { setOptions: jest.fn() };
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
  SettingsGroup: ({ children }: any) => children,
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
  servicesApi: { connection: jest.fn() },
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
  mockFocused = true;
  Socket.instances = [];
  mockInject.mockClear();
  global.WebSocket = Socket as unknown as typeof WebSocket;
  jest.mocked(servicesApi.connection).mockResolvedValue(connection);
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
const message = (view: any, data: unknown) =>
  fireEvent(view, "message", { nativeEvent: { data: JSON.stringify(data) } });

test("bearer stays in the RN socket header; inline HTML, bridge and every injected script stay token-free", async () => {
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  expect(view.props.source.html).not.toContain("SENTINEL_BEARER_TOKEN");
  expect(view.props.source.baseUrl).toBe("about:blank");
  expect(view.props.originWhitelist).toEqual(["about:blank"]);
  expect(view.props.allowFileAccess).toBe(false);
  await act(async () => {
    message(view, { type: "ready" });
  });
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
  await act(async () => {
    message(view, { type: "input", data: "ls\r" });
  });
  expect(socket.send).toHaveBeenCalled();
  expect(typeof socket.send.mock.calls[0]![0]).not.toBe("string");
  await fireEvent.press(screen.getByLabelText("ArrowUp"));
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

test("background and blur close the socket and require explicit reconnect; 401 retries once", async () => {
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  await act(async () => {
    message(screen.getByTestId("terminal-webview"), { type: "ready" });
  });
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
    mockAppHandlers.forEach((handler) => handler("background"));
  });
  expect(Socket.instances[2]!.close).toHaveBeenCalledWith(1000);
  expect(screen.getByText("Disconnected")).toBeTruthy();
  await act(async () => {
    mockAppHandlers.forEach((handler) => handler("active"));
  });
  expect(Socket.instances.length).toBe(3);
  mockFocused = false;
  await screen.rerender(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  await screen.unmount();
});

test("sticky Ctrl-C sends a binary interrupt once and releases the selected key", async () => {
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  await act(async () => {
    message(view, { type: "ready" });
  });
  await fireEvent.press(screen.getByLabelText("Ctrl"));
  expect(screen.getByLabelText("Ctrl").props.accessibilityState.selected).toBe(true);
  await act(async () => {
    message(view, { type: "input", data: "c" });
  });
  expect([...Socket.instances[0]!.send.mock.calls[0]![0]]).toEqual([3]);
  expect(screen.getByLabelText("Ctrl").props.accessibilityState.selected).toBe(false);
  await act(async () => {
    message(view, { type: "input", data: "c" });
  });
  expect([...Socket.instances[0]!.send.mock.calls[1]![0]]).toEqual([99]);
  await screen.unmount();
});

test("a terminated renderer gets a fresh inline document and navigation allowance", async () => {
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  const first = screen.getByTestId("terminal-webview");
  first.props.onLoadStart();
  await act(async () => {
    message(first, { type: "ready" });
  });
  expect(Socket.instances.length).toBe(1);
  await act(async () => {
    first.props.onContentProcessDidTerminate();
  });
  expect(Socket.instances[0]!.close).toHaveBeenCalledWith(1000);
  const replacement = screen.getByTestId("terminal-webview");
  expect(replacement.props.onShouldStartLoadWithRequest({ url: "about:blank", navigationType: "other" })).toBe(true);
  expect(replacement.props.onShouldStartLoadWithRequest({ url: "about:blank" })).toBe(false);
  expect(Socket.instances.length).toBe(1);
  await act(async () => {
    message(replacement, { type: "ready" });
  });
  expect(Socket.instances.length).toBe(2);
  await screen.unmount();
});

test.each(["The credential node is offline", "SSH host key mismatch"])(
  "server errors retain actionable node/host messages: %s",
  async (detail) => {
    const screen = await render(
      <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
    );
    await act(async () => {
      message(screen.getByTestId("terminal-webview"), { type: "ready" });
    });
    const socket = Socket.instances[0]!;
    await act(async () => {
      socket.onmessage?.({
        data: JSON.stringify({ type: "connected", cols: 80, rows: 24 }),
      });
    });
    expect(screen.getByText("Connected")).toBeTruthy();
    await act(async () => {
      socket.onmessage?.({
        data: JSON.stringify({ type: "error", message: detail }),
      });
    });
    expect(screen.getByText(detail)).toBeTruthy();
    expect(screen.getByText("Reconnect")).toBeTruthy();
    expect(socket.close).toHaveBeenCalledWith(1000);
    await screen.unmount();
  },
);

test("remote close and invalid bridge data never replay shell input; resizing is debounced", async () => {
  jest.useFakeTimers();
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  const view = screen.getByTestId("terminal-webview");
  await act(async () => {
    message(view, { type: "ready" });
  });
  const socket = Socket.instances[0]!;
  await act(async () => {
    message(view, { type: "input", data: "bad", url: "https://evil.example" });
    message(view, { type: "resize", cols: 1, rows: 2 });
  });
  expect(socket.send).not.toHaveBeenCalled();
  await act(async () => {
    message(view, { type: "resize", cols: 80, rows: 24 });
    message(view, { type: "resize", cols: 120, rows: 40 });
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
  await screen.unmount();
  jest.useRealTimers();
});

test("several principals require selection; a clipboard read resolving after blur cannot inject", async () => {
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    ssh_allowed_principals: ["ops", "deploy"],
  });
  const screen = await render(
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  await act(async () => {
    message(screen.getByTestId("terminal-webview"), { type: "ready" });
  });
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
    <SshTerminalScreen route={{ params: { serviceId: "connection" } }} />,
  );
  const injections = mockInject.mock.calls.length;
  await act(async () => {
    resolve("private clipboard");
  });
  expect(mockInject).toHaveBeenCalledTimes(injections);
  expect(Socket.instances[0]!.close).toHaveBeenCalledWith(1000);
  await screen.unmount();
});
