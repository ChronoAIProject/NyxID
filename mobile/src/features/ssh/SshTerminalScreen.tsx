import { useEffect, useMemo, useRef, useState } from "react";
import {
  AppState,
  Keyboard,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { WebView, type WebViewMessageEvent } from "react-native-webview";
import * as Clipboard from "expo-clipboard";
import * as WebBrowser from "expo-web-browser";
import { z } from "zod";
import { getApiBaseUrl } from "../../lib/api/baseUrl";
import { clientHeaders, refreshAccessTokenIfNeeded } from "../../lib/api/http";
import { loadStoredAuthSession } from "../../lib/auth/sessionStore";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../../lib/auth/authAccount";
import { getFrontendUrl } from "../../lib/env";
import { servicesApi } from "../../lib/api/servicesApi";
import type { ServiceConnection } from "../services/serviceContracts";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { terminalDocument } from "./terminalDocument";
import {
  controlInput,
  inlineTerminalNavigation,
  terminalBridgeSchema,
  terminalKeys,
  terminalScript,
  terminalUpgradeError,
  terminalUpgradeStatus,
  terminalUrl,
  type TerminalCommand,
} from "./terminalModel";
import { TerminalOutput } from "./terminalOutput";

// RN supports the native headers constructor argument; lib.dom's WebSocket
// declaration omits it. Keep that adaptation at the native transport boundary.
type NativeWebSocketConstructor = new (
  url: string,
  protocols: string[],
  options: { headers: Record<string, string> },
) => WebSocket;
const serverMessage = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("connected"),
    cols: z.number().optional(),
    rows: z.number().optional(),
  }),
  z.object({ type: z.literal("error"), message: z.string().max(8192) }),
  z.object({ type: z.literal("closed"), reason: z.string().optional() }),
]);
export function SshTerminalScreen({
  route,
}: {
  route: { params: { serviceId: string; principal?: string } };
}) {
  const { serviceId } = route.params;
  const focused = useIsFocused();
  const navigation = useNavigation();
  const { colors } = useTheme();
  const { t } = useAppLanguage();
  const [connection, setConnection] = useState<ServiceConnection>();
  const [principal, setPrincipal] = useState(route.params.principal);
  const [ready, setReady] = useState(false);
  const [state, setState] = useState("Connecting…");
  const [error, setError] = useState<string>();
  const [ctrl, setCtrl] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const web = useRef<WebView>(null);
  const socket = useRef<WebSocket | null>(null);
  const size = useRef({ cols: 80, rows: 24 });
  const generation = useRef(0);
  const resizeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const html = useMemo(
    () =>
      terminalDocument({
        background: colors.bg,
        foreground: colors.textPrimary,
        cursor: colors.primary,
      }),
    [colors.bg, colors.textPrimary, colors.primary],
  );
  const allowNavigation = useMemo(() => inlineTerminalNavigation(), [html]);
  const send = (message: TerminalCommand) =>
    web.current?.injectJavaScript(terminalScript(message));
  const output = useMemo(
    () =>
      new TerminalOutput(
        (data, id) => send({ type: "output", data, id }),
        () => {
          socket.current?.close(1000);
          setError("Terminal output is too far behind. Reconnect to continue.");
          setState("Disconnected");
        },
      ),
    [],
  );
  useEffect(() => {
    setReady(false);
    output.clear();
  }, [html, output]);

  useEffect(() => {
    navigation.setOptions({ title: t("SSH terminal") });
  }, [navigation, t]);
  useEffect(() => {
    let current = true;
    const account = getAuthAccountId();
    setConnection(undefined);
    void servicesApi
      .connection(serviceId)
      .then((value) => {
        if (!current || account !== getAuthAccountId()) return;
        setConnection(value);
        if (
          !route.params.principal &&
          value.ssh_allowed_principals?.length === 1
        )
          setPrincipal(value.ssh_allowed_principals[0]);
      })
      .catch(() => {
        if (current)
          setError(
            "This SSH connection could not be loaded. Try reconnecting.",
          );
      });
    return () => {
      current = false;
    };
  }, [serviceId, attempt, route.params.principal]);

  const eligible =
    connection?.service_type === "ssh" &&
    connection.ssh_auth_mode === "cert" &&
    connection.is_active &&
    connection.status === "active";
  useEffect(() => {
    const epoch = ++generation.current;
    const account = getAuthAccountId();
    if (
      !focused ||
      !ready ||
      !eligible ||
      !principal ||
      !account ||
      !connection?.ssh_allowed_principals?.includes(principal)
    )
      return;
    let closed = false;
    let retries = 0;
    const current = () =>
      !closed && epoch === generation.current && account === getAuthAccountId();
    const close = () => {
      closed = true;
      generation.current++;
      output.clear();
      clearTimeout(resizeTimer.current);
      const ws = socket.current;
      socket.current = null;
      if (ws) {
        ws.onmessage = null;
        ws.onclose = null;
        ws.onerror = null;
        ws.onopen = null;
        ws.close(1000);
      }
    };
    const connect = async (force = false) => {
      setState("Connecting…");
      setError(undefined);
      try {
        await refreshAccessTokenIfNeeded(force);
        const session = await loadStoredAuthSession();
        if (!current()) return;
        if (!session?.accessToken) {
          setError(terminalUpgradeError(401));
          setState("Disconnected");
          return;
        }
        const ws = new (WebSocket as unknown as NativeWebSocketConstructor)(
          terminalUrl(
            getApiBaseUrl(),
            connection.catalog_service_id ?? serviceId,
            principal,
            size.current.cols,
            size.current.rows,
          ),
          [],
          {
            headers: {
              ...clientHeaders(),
              Authorization: `Bearer ${session.accessToken}`,
            },
          },
        );
        socket.current = ws;
        ws.binaryType = "arraybuffer";
        let transportError = false;
        const fail = (status?: number) => {
          socket.current = null;
          output.clear();
          ws.onclose = null;
          ws.onmessage = null;
          ws.onerror = null;
          ws.close(1000);
          if (status === 401 && retries++ === 0) {
            void connect(true);
            return;
          }
          setError(terminalUpgradeError(status));
          setState("Disconnected");
        };
        ws.onmessage = (event) => {
          if (!current() || socket.current !== ws) return;
          if (typeof event.data !== "string") {
            if (event.data instanceof ArrayBuffer) output.push(event.data);
            return;
          }
          try {
            const message = serverMessage.parse(JSON.parse(event.data));
            if (message.type === "connected") setState("Connected");
            if (message.type === "error") {
              setError(message.message);
              setState("Disconnected");
              ws.close(1000);
            }
            if (message.type === "closed") {
              setState("Closed by remote host");
              ws.close(1000);
            }
          } catch {
            setError("The terminal server sent an invalid message.");
            ws.close(1000);
          }
        };
        ws.onerror = (event) => {
          if (!current() || socket.current !== ws) return;
          const status = terminalUpgradeStatus(
            event as unknown as { message?: string; status?: number },
          );
          transportError = true;
          // RN 0.83 emits an empty error Event, then a CloseEvent whose reason
          // contains the native upgrade failure. Preserve that second event.
          if (status) fail(status);
        };
        ws.onclose = (event) => {
          if (!current() || socket.current !== ws) return;
          const status = terminalUpgradeStatus({ message: event?.reason });
          if (transportError || status) {
            fail(status);
            return;
          }
          socket.current = null;
          output.clear();
          setState((value) =>
            value === "Closed by remote host" ? value : "Disconnected",
          );
        };
      } catch {
        if (current()) {
          setError(terminalUpgradeError());
          setState("Disconnected");
        }
      }
    };
    void connect();
    const appState = AppState.addEventListener("change", (value) => {
      if (value !== "active") {
        close();
        setState("Disconnected");
      }
    });
    const accountSubscription = subscribeAuthAccount(() => {
      close();
      setState("Disconnected");
      setConnection(undefined);
    });
    return () => {
      close();
      appState.remove();
      accountSubscription();
    };
  }, [
    focused,
    ready,
    eligible,
    principal,
    serviceId,
    attempt,
    connection,
    output,
  ]);

  const bridge = (event: WebViewMessageEvent) => {
    let message;
    try {
      message = terminalBridgeSchema.parse(JSON.parse(event.nativeEvent.data));
    } catch {
      return;
    }
    if (message.type === "ready") {
      setReady(true);
      return;
    }
    if (message.type === "ack") {
      output.ack(message.id);
      return;
    }
    if (
      message.type === "input" &&
      socket.current?.readyState === WebSocket.OPEN
    ) {
      socket.current.send(
        new TextEncoder().encode(
          ctrl ? controlInput(message.data) : message.data,
        ),
      );
      if (ctrl) setCtrl(false);
    }
    if (message.type === "resize") {
      size.current = { cols: message.cols, rows: message.rows };
      clearTimeout(resizeTimer.current);
      resizeTimer.current = setTimeout(() => {
        if (socket.current?.readyState === WebSocket.OPEN)
          socket.current.send(
            JSON.stringify({ type: "resize", ...size.current }),
          );
      }, 100);
    }
  };
  useEffect(() => {
    const subscriptions = ["keyboardDidShow", "keyboardDidHide"].map((event) =>
      Keyboard.addListener(event as "keyboardDidShow" | "keyboardDidHide", () =>
        send({ type: "fit" }),
      ),
    );
    return () => subscriptions.forEach((subscription) => subscription.remove());
  }, []);
  const reconnect = () => {
    setReady(true);
    setAttempt((value) => value + 1);
  };
  const paste = async () => {
    const epoch = generation.current;
    const account = getAuthAccountId();
    try {
      const text = await Clipboard.getStringAsync();
      if (
        focused &&
        epoch === generation.current &&
        account === getAuthAccountId()
      )
        send({ type: "paste", text: text.slice(0, 65536) });
    } catch {
      if (epoch === generation.current)
        setError("Couldn't read the clipboard. Try again.");
    }
  };
  return (
    <WorkspacePage
      title={t("SSH terminal")}
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      {error && <Notice message={t(error)} tone="error" />}
      {connection && !eligible ? (
        <SettingsGroup
          footer={t(
            "The mobile terminal is available for active certificate-mode SSH services. Use the web terminal or nyxid ssh exec for other modes.",
          )}
        >
          <SettingsRow
            label={t("Open web terminal")}
            tone="action"
            onPress={() =>
              void WebBrowser.openBrowserAsync(
                new URL(
                  `/keys/${encodeURIComponent(serviceId)}`,
                  getFrontendUrl(),
                ).href,
              ).catch(() => setError(terminalUpgradeError()))
            }
          />
        </SettingsGroup>
      ) : (
        <>
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.status, { color: colors.textSecondary }]}
          >
            {t(
              eligible && !principal
                ? connection?.ssh_allowed_principals?.length
                  ? "Choose an SSH principal to connect."
                  : "No SSH principals are configured for this service."
                : state,
            )}
          </Text>
          {connection?.has_node_binding === false && (
            <Notice
              message={t(
                "Bind an online credential node to this SSH service before opening a terminal.",
              )}
            />
          )}
          {(connection?.ssh_allowed_principals?.length ?? 0) > 1 && (
            <SettingsGroup title={t("SSH principal")}>
              {connection!.ssh_allowed_principals!.map((value) => (
                <SettingsRow
                  key={value}
                  label={value}
                  value={principal === value ? t("Selected") : undefined}
                  onPress={() => setPrincipal(value)}
                />
              ))}
            </SettingsGroup>
          )}
          <View style={styles.terminal} onLayout={() => send({ type: "fit" })}>
            <WebView
              ref={web}
              source={{ html, baseUrl: "about:blank" }}
              originWhitelist={["about:blank"]}
              onLoadStart={allowNavigation.loaded}
              onShouldStartLoadWithRequest={allowNavigation}
              onOpenWindow={() => {}}
              javaScriptCanOpenWindowsAutomatically={false}
              allowFileAccess={false}
              allowFileAccessFromFileURLs={false}
              allowUniversalAccessFromFileURLs={false}
              mixedContentMode="never"
              dataDetectorTypes="none"
              sharedCookiesEnabled={false}
              thirdPartyCookiesEnabled={false}
              domStorageEnabled={false}
              onMessage={bridge}
              onContentProcessDidTerminate={() => {
                socket.current?.close(1000);
                setState("Disconnected");
                setReady(false);
                web.current?.reload();
              }}
              style={{ backgroundColor: colors.bg }}
            />
          </View>
          <ScrollView
            horizontal
            keyboardShouldPersistTaps="always"
            style={styles.keys}
            contentContainerStyle={{ gap: spacing.xs }}
          >
            {[...terminalKeys, "Ctrl", "Paste"].map((key) => (
              <Pressable
                key={key}
                accessibilityRole="button"
                accessibilityLabel={t(key)}
                accessibilityState={{ selected: key === "Ctrl" && ctrl }}
                style={[
                  styles.key,
                  {
                    backgroundColor: colors.card,
                    borderColor:
                      key === "Ctrl" && ctrl ? colors.primary : colors.border,
                  },
                ]}
                onPress={() => {
                  if (key === "Ctrl") setCtrl((value) => !value);
                  else if (key === "Paste") void paste();
                  else
                    send({
                      type: "key",
                      key: key as (typeof terminalKeys)[number],
                    });
                }}
              >
                <Text
                  style={[
                    styles.keyText,
                    {
                      color:
                        key === "Ctrl" && ctrl
                          ? colors.primary
                          : colors.textPrimary,
                    },
                  ]}
                >
                  {t(
                    (
                      {
                        ArrowUp: "↑",
                        ArrowDown: "↓",
                        ArrowLeft: "←",
                        ArrowRight: "→",
                      } as Record<string, string>
                    )[key] ?? key,
                  )}
                </Text>
              </Pressable>
            ))}
          </ScrollView>
        </>
      )}
      {(state === "Disconnected" ||
        state === "Closed by remote host" ||
        error) && (
        <SettingsRow label={t("Reconnect")} tone="action" onPress={reconnect} />
      )}
    </WorkspacePage>
  );
}
const styles = StyleSheet.create({
  page: { padding: spacing.sm, gap: spacing.sm },
  terminal: { flex: 1, minHeight: 100 },
  status: { ...typeScale.label },
  keys: { flexGrow: 0, maxHeight: 48 },
  key: {
    borderWidth: 1,
    minWidth: 44,
    minHeight: 44,
    justifyContent: "center",
    alignItems: "center",
    borderRadius: 6,
    paddingHorizontal: spacing.sm,
  },
  keyText: { ...typeScale.mono },
});
