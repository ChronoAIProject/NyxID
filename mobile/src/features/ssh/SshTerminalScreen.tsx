import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  AppState,
  Keyboard,
  KeyboardAvoidingView,
  Pressable,
  Platform,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { WebView, type WebViewMessageEvent } from "react-native-webview";
import * as Clipboard from "expo-clipboard";
import { z } from "zod";
import { getApiBaseUrl } from "../../lib/api/baseUrl";
import { clientHeaders, refreshAccessTokenIfNeeded } from "../../lib/api/http";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { loadStoredAuthSession } from "../../lib/auth/sessionStore";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../../lib/auth/authAccount";
import { servicesApi } from "../../lib/api/servicesApi";
import type { ServiceConnection } from "../services/serviceContracts";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { PrimaryButton } from "../../components/PrimaryButton";
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
  truncateTerminalPaste,
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
const TERMINAL_BACKGROUND_GRACE_MS = 15_000;
export function SshTerminalScreen({
  route,
}: {
  route: {
    params: {
      serviceId: string;
      connectionId?: string;
      principal?: string;
      autoConnect?: boolean;
    };
  };
}) {
  const { serviceId, connectionId } = route.params;
  const focused = useIsFocused();
  const focusedRef = useRef(focused);
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const { colors } = useTheme();
  const { t } = useAppLanguage();
  const translateRef = useRef(t);
  translateRef.current = t;
  const [connection, setConnection] = useState<ServiceConnection>();
  const [notConnected, setNotConnected] = useState(false);
  const [loadUnavailable, setLoadUnavailable] = useState(false);
  const [principal, setPrincipal] = useState(route.params.principal);
  const principalRef = useRef(principal);
  const lastAppliedRoutePrincipal = useRef(route.params.principal);
  const [principalPickerOpen, setPrincipalPickerOpen] = useState(false);
  const [ready, setReady] = useState(false);
  const [state, setState] = useState("Connecting…");
  const [error, setError] = useLocalizedState<string>();
  const [notice, setNotice] = useLocalizedState<string>();
  const [waiting, setWaiting] = useState(false);
  const waitingRef = useRef(false);
  const sessionEndedRef = useRef(false);
  const [ctrl, setCtrl] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [connectRequested, setConnectRequested] = useState(
    route.params.autoConnect === true,
  );
  const [documentVersion, setDocumentVersion] = useState(0);
  const web = useRef<WebView>(null);
  const socket = useRef<WebSocket | null>(null);
  const size = useRef({ cols: 80, rows: 24 });
  const generation = useRef(0);
  const resizeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const waitingTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const backgroundTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const backgroundStartedAt = useRef<number | undefined>(undefined);
  const html = useMemo(
    () => terminalDocument(),
    [],
  );
  const allowNavigation = useMemo(
    () => inlineTerminalNavigation(),
    [html, documentVersion],
  );
  const send = (message: TerminalCommand) =>
    web.current?.injectJavaScript(terminalScript(message));
  const stopWaiting = () => {
    clearTimeout(waitingTimer.current);
    waitingRef.current = false;
    setWaiting(false);
  };
  const endSession = (
    message = translateRef.current("Connection closed."),
  ) => {
    if (sessionEndedRef.current) return;
    sessionEndedRef.current = true;
    send({ type: "session-ended", message });
  };
  const resetForAttempt = () => {
    sessionEndedRef.current = false;
    setNotice(undefined);
    stopWaiting();
  };
  const output = useMemo(
    () =>
      new TerminalOutput(
        (data, id) => send({ type: "output", data, id }),
        () =>
          new TextEncoder().encode(
            `\x18\x1b[0m\r\n${translateRef.current("Output skipped.")}\r\n`,
          ),
      ),
    [],
  );
  useEffect(() => {
    setReady(false);
    output.clear();
  }, [html, documentVersion, output]);

  useEffect(() => {
    focusedRef.current = focused;
  }, [focused]);
  useEffect(() => {
    principalRef.current = principal;
  }, [principal]);
  useEffect(() => {
    setConnectRequested(route.params.autoConnect === true);
  }, [connectionId, route.params.autoConnect, serviceId]);
  useEffect(() => {
    navigation.setOptions({ title: t("SSH terminal") });
  }, [navigation, t]);
  useEffect(() => {
    let current = true;
    const account = getAuthAccountId();
    setConnection(undefined);
    setNotConnected(false);
    setLoadUnavailable(false);
    setError(undefined);
    setState("Connecting…");
    const load = connectionId
      ? servicesApi.connection(connectionId)
      : servicesApi
          .connections()
          .then((values) =>
            values.find(
              (value) =>
                value.catalog_service_id === serviceId &&
                value.service_type === "ssh" &&
                value.is_active &&
                value.status === "active",
            ),
          );
    void load
      .then((value) => {
        if (!current || account !== getAuthAccountId()) return;
        if (!value) {
          setNotConnected(true);
          return;
        }
        setConnection(value);
        const allowedPrincipals = value.ssh_allowed_principals ?? [];
        const requestedPrincipal = route.params.principal;
        const selectedPrincipal = principalRef.current;
        const routeChanged =
          lastAppliedRoutePrincipal.current !== requestedPrincipal;
        lastAppliedRoutePrincipal.current = requestedPrincipal;
        const allowedRoute =
          requestedPrincipal && allowedPrincipals.includes(requestedPrincipal)
            ? requestedPrincipal
            : undefined;
        const allowedSelection =
          selectedPrincipal && allowedPrincipals.includes(selectedPrincipal)
            ? selectedPrincipal
            : undefined;
        const nextPrincipal =
          (routeChanged ? allowedRoute : undefined) ??
          allowedSelection ??
          allowedRoute ??
          (allowedPrincipals.length === 1 ? allowedPrincipals[0] : undefined);
        setPrincipal(nextPrincipal);
        setPrincipalPickerOpen(!nextPrincipal && allowedPrincipals.length > 0);
      })
      .catch(() => {
        if (current) {
          setLoadUnavailable(true);
          setState("Unavailable");
          setError(undefined);
        }
      });
    return () => {
      current = false;
    };
  }, [serviceId, connectionId, attempt, route.params.principal]);

  const eligible =
    connection?.service_type === "ssh" &&
    connection.ssh_auth_mode === "cert" &&
    connection.is_active &&
    connection.status === "active";
  useEffect(() => {
    const epoch = ++generation.current;
    const account = getAuthAccountId();
    if (
      !ready ||
      !connectRequested ||
      !eligible ||
      !principal ||
      !account ||
      !connection?.ssh_allowed_principals?.includes(principal)
    )
      return;
    let closed = false;
    let deferredConnect = AppState.currentState !== "active";
    let retries = 0;
    const current = () =>
      !closed && epoch === generation.current && account === getAuthAccountId();
    const close = (notice?: string) => {
      closed = true;
      generation.current++;
      if (notice) endSession(notice);
      output.clear();
      clearTimeout(resizeTimer.current);
      clearTimeout(backgroundTimer.current);
      backgroundTimer.current = undefined;
      backgroundStartedAt.current = undefined;
      stopWaiting();
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
      resetForAttempt();
      waitingTimer.current = setTimeout(() => {
        if (current()) {
          waitingRef.current = true;
          setWaiting(true);
        }
      }, 2000);
      try {
        await refreshAccessTokenIfNeeded(force);
        const session = await loadStoredAuthSession();
        if (!current()) return;
        if (AppState.currentState !== "active") {
          deferredConnect = true;
          if (backgroundStartedAt.current === undefined) startBackgroundGrace();
          return;
        }
        if (!session?.accessToken) {
          stopWaiting();
          endSession();
          setError(() => (terminalUpgradeError(401)));
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
        let terminalSettled = false;
        const fail = (status?: number) => {
          if (terminalSettled) return;
          const waitingForApproval = waitingRef.current;
          stopWaiting();
          endSession();
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
          setError(() => (terminalUpgradeError(status, waitingForApproval)));
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
            if (message.type === "connected") {
              resetForAttempt();
              send({ type: "reset" });
              setState("Connected");
            }
            if (message.type === "error") {
              terminalSettled = true;
              stopWaiting();
              endSession(message.message);
              setError(() => (message.message));
              setState("Disconnected");
              socket.current = null;
              output.clear();
              ws.close(1000);
            }
            if (message.type === "closed") {
              terminalSettled = true;
              stopWaiting();
              endSession(
                translateRef.current("Session closed by remote host."),
              );
              setState("Closed by remote host");
              socket.current = null;
              output.clear();
              ws.close(1000);
            }
          } catch {
            terminalSettled = true;
            stopWaiting();
            endSession();
            setError(() => ("The terminal server sent an invalid message."));
            setState("Disconnected");
            socket.current = null;
            output.clear();
            ws.close(1000);
          }
        };
        ws.onerror = (event) => {
          if (terminalSettled || !current() || socket.current !== ws) return;
          const status = terminalUpgradeStatus(
            event as unknown as { message?: string; status?: number },
          );
          transportError = true;
          // RN 0.83 emits an empty error Event, then a CloseEvent whose reason
          // contains the native upgrade failure. Preserve that second event.
          if (status) fail(status);
        };
        ws.onclose = (event) => {
          if (terminalSettled || !current() || socket.current !== ws) return;
          const status = terminalUpgradeStatus({ message: event?.reason });
          if (transportError || status) {
            fail(status);
            return;
          }
          stopWaiting();
          endSession();
          socket.current = null;
          output.clear();
          setState((value) =>
            value === "Closed by remote host" ? value : "Disconnected",
          );
        };
      } catch {
        if (current()) {
          stopWaiting();
          endSession();
          setError(() => (terminalUpgradeError()));
          setState("Disconnected");
        }
      }
    };
    const startBackgroundGrace = () => {
      clearTimeout(backgroundTimer.current);
      backgroundStartedAt.current = Date.now();
      backgroundTimer.current = setTimeout(() => {
        backgroundTimer.current = undefined;
        backgroundStartedAt.current = undefined;
        close(translateRef.current("Connection closed."));
        setState("Disconnected");
      }, TERMINAL_BACKGROUND_GRACE_MS);
    };
    if (deferredConnect) startBackgroundGrace();
    else void connect();
    const appState = AppState.addEventListener("change", (value) => {
      if (value === "background") {
        startBackgroundGrace();
      } else if (value === "active") {
        const startedAt = backgroundStartedAt.current;
        if (
          startedAt !== undefined &&
          Date.now() - startedAt >= TERMINAL_BACKGROUND_GRACE_MS
        ) {
          clearTimeout(backgroundTimer.current);
          backgroundTimer.current = undefined;
          backgroundStartedAt.current = undefined;
          close(translateRef.current("Connection closed."));
          setState("Disconnected");
        } else {
          clearTimeout(backgroundTimer.current);
          backgroundTimer.current = undefined;
          backgroundStartedAt.current = undefined;
          if (deferredConnect && current()) {
            deferredConnect = false;
            void connect();
          }
        }
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
    ready,
    connectRequested,
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
      !sessionEndedRef.current &&
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
    setNotice(undefined);
    setConnectRequested(true);
    endSession();
    generation.current++;
    const ws = socket.current;
    socket.current = null;
    if (ws) {
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.onopen = null;
      ws.close(1000);
    }
    output.clear();
    stopWaiting();
    setAttempt((value) => value + 1);
  };
  const paste = async () => {
    setNotice(undefined);
    const epoch = generation.current;
    const account = getAuthAccountId();
    try {
      const text = await Clipboard.getStringAsync();
      const pasted = truncateTerminalPaste(text);
      const sent =
        focusedRef.current &&
        epoch === generation.current &&
        account === getAuthAccountId()
      if (sent) {
        send({ type: "paste", text: pasted.text });
        if (pasted.truncated)
          setNotice(() => ("Only the first 65,536 characters were pasted."));
      }
    } catch {
      if (epoch === generation.current)
        setError(() => ("Couldn't read the clipboard. Try again."));
    }
  };
  const handleRendererTerminated = () => {
    const ws = socket.current;
    socket.current = null;
    if (ws) {
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.close(1000);
    }
    output.clear();
    stopWaiting();
    endSession();
    setState("Disconnected");
    setReady(false);
    setConnectRequested(false);
    setNotice(() => ("The terminal display stopped. Connect again to continue."));
    setDocumentVersion((value) => value + 1);
  };
  const principals = connection?.ssh_allowed_principals ?? [];
  const invalidPrincipal = Boolean(
    route.params.principal && !principal && principals.length,
  );
  const statusMessage = waiting
    ? "Waiting for approval or the node…"
    : connection?.has_node_binding === false
      ? "Bind an online credential node to this SSH service before opening a terminal."
      : eligible && invalidPrincipal
        ? "That SSH principal is not allowed for this service. Choose one below."
        : eligible && !principal
          ? principals.length
            ? "Choose an SSH principal to connect."
            : "No SSH principals are configured for this service."
          : state;
  return (
    <WorkspacePage
      title={t("SSH terminal")}
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      {loadUnavailable || notConnected ? (
        <SettingsGroup
          footer={t(
            loadUnavailable
              ? "This SSH connection could not be loaded."
              : "This SSH service is not connected to your account. Open Services to connect it first.",
          )}
        >
          <SettingsRow
            label={t("Open Services")}
            tone="action"
            onPress={() => navigation.navigate("Services")}
          />
        </SettingsGroup>
      ) : connection && !eligible ? (
        <View style={styles.unavailable}>
          <Notice
            message={t(
              connection.ssh_auth_mode === "node_key"
                ? "Certificate-mode services are required for the mobile terminal. Run nyxid ssh exec <service> from a computer."
                : connection.ssh_auth_mode === "proxy_only"
                  ? "Certificate-mode services are required for the mobile terminal. Run nyxid ssh proxy from a computer."
                  : "Enable this connection to open the terminal.",
            )}
          />
        </View>
      ) : connection && eligible && !connectRequested ? (
        <View style={styles.preconnect}>
          {notice ? <Notice message={t(notice)} /> : null}
          <Notice
            message={
              principal
                ? t("Connect to {{connection}} as {{principal}}?", {
                    connection: connection.label,
                    principal,
                  })
                : t("Choose an SSH principal, then connect to {{connection}}.", {
                    connection: connection.label,
                  })
            }
          />
          {principals.length > 0 ? (
            <SettingsGroup>
              <SettingsRow
                label={t("SSH principal")}
                value={principal ?? t("Choose principal")}
                onPress={
                  principals.length > 1
                    ? () => setPrincipalPickerOpen((value) => !value)
                    : undefined
                }
              />
            </SettingsGroup>
          ) : null}
          {principalPickerOpen && principals.length > 0 ? (
            <SettingsGroup title={t("SSH principal")}>
              {principals.map((value) => (
                <SettingsRow
                  key={value}
                  label={value}
                  value={principal === value ? t("Selected") : undefined}
                  onPress={() => {
                    setPrincipal(value);
                    setPrincipalPickerOpen(false);
                  }}
                />
              ))}
            </SettingsGroup>
          ) : null}
          <PrimaryButton
            label={t("Connect")}
            accessibilityLabel={t("Connect")}
            disabled={!principal}
            onPress={() => {
              setNotice(undefined);
              setConnectRequested(true);
            }}
          />
        </View>
      ) : (
        <KeyboardAvoidingView
          style={styles.keyboardAvoiding}
          behavior={Platform.OS === "ios" ? "padding" : "height"}
        >
          <View style={styles.statusBar}>
            {principals.length > 1 || invalidPrincipal ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("SSH principal")}
                onPress={() => setPrincipalPickerOpen((value) => !value)}
                style={[styles.principalChip, { borderColor: colors.border }]}
              >
                <Text style={[styles.chipText, { color: colors.textPrimary }]}>
                  {principal ?? t("Choose principal")}
                </Text>
              </Pressable>
            ) : null}
            <Text
              accessibilityLiveRegion="polite"
              style={[styles.status, { color: colors.textSecondary }]}
              numberOfLines={2}
            >
              {t(statusMessage)}
            </Text>
            {(state === "Disconnected" ||
              state === "Closed by remote host" ||
              Boolean(error)) && (
              <Pressable
                accessibilityRole="button"
                onPress={reconnect}
                style={styles.statusAction}
              >
                <Text
                  style={[styles.statusActionText, { color: colors.primary }]}
                >
                  {t("Reconnect")}
                </Text>
              </Pressable>
            )}
            {waiting && (
              <Pressable
                accessibilityRole="button"
                onPress={() => navigation.push("ActivityReview", undefined)}
                style={styles.statusAction}
              >
                <Text
                  style={[styles.statusActionText, { color: colors.primary }]}
                >
                  {t("Review approvals")}
                </Text>
              </Pressable>
            )}
          </View>
          {error ? (
            <Text
              accessibilityRole="alert"
              accessibilityLiveRegion="polite"
              style={[styles.error, { color: colors.textPrimary }]}
            >
              {t(error)}
            </Text>
          ) : null}
          {notice ? <Notice message={t(notice)} /> : null}
          {principalPickerOpen && principals.length > 0 ? (
            <SettingsGroup title={t("SSH principal")}>
              {principals.map((value) => (
                <SettingsRow
                  key={value}
                  label={value}
                  value={principal === value ? t("Selected") : undefined}
                  onPress={() => {
                    setPrincipal(value);
                    setPrincipalPickerOpen(false);
                  }}
                />
              ))}
            </SettingsGroup>
          ) : null}
          <View style={styles.terminal} onLayout={() => send({ type: "fit" })}>
            <WebView
              key={documentVersion}
              ref={web}
              source={{ html, baseUrl: "about:blank" }}
              originWhitelist={["about:blank"]}
              hideKeyboardAccessoryView
              keyboardDisplayRequiresUserAction
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
              onContentProcessDidTerminate={handleRendererTerminated}
              onRenderProcessGone={handleRendererTerminated}
              style={{ backgroundColor: "#0f172a" }}
            />
          </View>
          <ScrollView
            horizontal
            keyboardShouldPersistTaps="always"
            style={styles.keys}
            contentContainerStyle={{ gap: spacing.xs }}
          >
            {["Ctrl", ...terminalKeys, "Paste"].map((key) => (
              <Pressable
                key={key}
                accessibilityRole="button"
                accessibilityLabel={t(
                  key === "Ctrl"
                    ? ctrl
                      ? "Control (armed)"
                      : "Control"
                    : key === "Esc"
                      ? "Escape"
                      : key === "Tab"
                        ? "Tab"
                      : key === "ArrowUp"
                        ? "Up arrow"
                        : key === "ArrowDown"
                          ? "Down arrow"
                          : key === "ArrowLeft"
                            ? "Left arrow"
                            : key === "ArrowRight"
                              ? "Right arrow"
                              : key === "|"
                                ? "Vertical bar"
                                : key === "~"
                                  ? "Tilde"
                                  : key === "/"
                                    ? "Slash"
                                    : key === "-"
                                      ? "Hyphen"
                                      : "Paste",
                )}
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
        </KeyboardAvoidingView>
      )}
    </WorkspacePage>
  );
}
const styles = StyleSheet.create({
  page: { padding: spacing.sm, gap: spacing.sm, flex: 1 },
  keyboardAvoiding: { flex: 1, gap: spacing.xs },
  unavailable: { gap: spacing.sm },
  preconnect: { gap: spacing.sm },
  statusBar: {
    minHeight: 38,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  principalChip: {
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
  },
  chipText: { ...typeScale.label },
  terminal: { flex: 1, minHeight: 100, backgroundColor: "#0f172a" },
  status: { ...typeScale.label, flex: 1 },
  error: { ...typeScale.label, alignSelf: "stretch" },
  statusAction: {
    flexShrink: 0,
    paddingHorizontal: spacing.xs,
    paddingVertical: spacing.xs,
  },
  statusActionText: { ...typeScale.label, fontWeight: "600" },
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
