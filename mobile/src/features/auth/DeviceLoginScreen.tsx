import { useLocalizedState } from "../../i18n/useLocalizedState";
import { Translation } from "../../i18n/Translation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { LoginRequestTerminalView } from "./LoginRequestTerminalView";
import { useUncertainLoginPreview } from "./useUncertainLoginPreview";
import { useLoginRequestResume } from "./useLoginRequestResume";
import { useLoginPreviewCooldown } from "./useLoginPreviewCooldown";
import { useLoginRequestHandoff } from "./useLoginRequestHandoff";
import {
  takeLoginRequest,
  resolveLoginRequest,
  previewLoginRequest,
  previewCooldownMs,
  deviceLoginErrorCopy,
  classifyLoginError,
  isLoginDecisionUncertain,
  loginServerHost,
  terminalFromError,
  terminalFromPreview,
  type LoginTerminal,
} from "./loginRequestResolver";
import { AuthSubmitButton } from "./AuthSubmitButton";
import { useMfaStepUp } from "./useMfaStepUp";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useWorkspaceBack, useWorkspaceSheet } from "../../app/WorkspaceHeader";
import {
  ActivityIndicator,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { KeyboardViewport } from "../../components/workspace/KeyboardViewport";
import {
  ChevronDown,
  ChevronLeft,
  ChevronUp,
  Info,
  ScanQrCode,
  X,
} from "lucide-react-native";

import { StackActions } from "@react-navigation/routers";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { workspaceNavigateAction } from "../../app/navigationHierarchy";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { mobileApi } from "../../lib/api/mobileApi";
import type { AuthDevicePreview } from "../../lib/api/authDeviceApi";
import { spacing } from "../../theme/designTokens";
import { useAuthTheme as useTheme } from "./useAuthTheme";
import { AuthTextInput } from "./AuthTextInput";
import { DeviceCodeScanner } from "./DeviceCodeScanner";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import {
  compareDeviceLoginTimezones,
  formatDeviceLoginOriginValue,
  formatDeviceLoginRelativeTime,
  resolveDeviceLoginDeadlineMs,
  resolveDeviceLoginValueTones,
  secondsUntilDeviceLoginDeadline,
} from "./deviceLoginPreview";
import {
  formatAuthDeviceUserCode,
  normalizeAuthDeviceUserCode,
} from "./deviceUserCode";
import { useAuthSession } from "./AuthSessionContext";

type Props = SheetScreenProps<"DeviceLogin">;
type TerminalState = LoginTerminal | null;

const ACTION_THROTTLE_MS = 750;
function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? translateApp("Unknown time")
    : parsed.toLocaleString(appLocale());
}

function formatRemaining(seconds: number): string {
  const safeSeconds = Math.max(0, seconds);
  const minutes = Math.floor(safeSeconds / 60);
  const remainder = safeSeconds % 60;
  return `${minutes}:${String(remainder).padStart(2, "0")}`;
}

export function DetailRow({
  label,
  value,
  mono = false,
  tone = "default",
  styles,
}: {
  label: string;
  value: string;
  mono?: boolean;
  tone?: "default" | "warning" | "danger";
  styles: ReturnType<typeof createDeviceLoginStyles>;
}) {
  useAppLanguage();
  const toneStyle =
    tone === "warning"
      ? styles.detailValueWarning
      : tone === "danger"
        ? styles.detailValueDanger
        : null;
  return (
    <View style={styles.detailRow}>
      <Text style={styles.detailLabel}>{label}</Text>
      <Text
        style={[
          styles.detailValue,
          mono ? styles.detailValueMono : null,
          toneStyle,
        ]}
      >
        {value}
      </Text>
    </View>
  );
}

export function formatLocation(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution !== "verified") return translateApp("Not available");
  const locality = [preview.client_city, preview.client_region]
    .filter((value): value is string => Boolean(value))
    .join(", ");
  const place =
    locality && preview.client_country
      ? `${locality} (${preview.client_country})`
      : locality || preview.client_country || preview.client_continent;
  if (place && preview.client_ip_timezone) {
    return `${place} · ${preview.client_ip_timezone}`;
  }
  if (place) return place;
  return preview.client_ip_timezone
    ? translateApp("IP timezone: {{clientIpTimezone}}", {
        clientIpTimezone: preview.client_ip_timezone,
      })
    : translateApp("Not available");
}

export function formatNetwork(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution !== "verified") return translateApp("Not available");
  const relation =
    preview.network_relation ??
    (preview.same_ip_as_viewer === true
      ? "same_ip"
      : preview.same_ip_as_viewer === false
        ? "different_ip"
        : null);
  if (relation === "same_ip") return translateApp("Same IP as this phone");
  if (relation === "same_network") return translateApp("Same network as this phone");
  if (relation === "different_network") return translateApp("Different network");
  if (relation === "different_ip") return translateApp("Different IP");
  return translateApp("Not available");
}

function formatScreen(preview: AuthDevicePreview): string {
  if (
    preview.client_screen_width === null ||
    preview.client_screen_height === null
  ) {
    return translateApp("Not reported");
  }
  return translateApp(
    preview.client_device_pixel_ratio === null
      ? "{{width}} x {{height}} CSS px"
      : "{{width}} x {{height}} CSS px at {{scale}}x",
    { width: preview.client_screen_width, height: preview.client_screen_height, scale: preview.client_device_pixel_ratio ?? 1 },
  );
}

export function formatDevice(preview: AuthDevicePreview): string {
  if (preview.client_label && preview.client_model) {
    return `${preview.client_label} · ${preview.client_model}`;
  }
  return (
    preview.client_label ?? preview.client_model ?? translateApp("Not provided")
  );
}

function timezoneRow(
  preview: AuthDevicePreview,
  localTimezone: string | null,
): { value: string; anomalous: boolean } {
  if (!preview.client_timezone)
    return { value: translateApp("Not reported"), anomalous: false };
  const differsFromPhone =
    compareDeviceLoginTimezones(preview.client_timezone, localTimezone) ===
    "different";
  const differences = [
    differsFromPhone ? translateApp("this phone") : null,
    preview.client_timezone_matches_ip === false ? translateApp("IP location")
      : null,
  ].filter((value): value is string => value !== null);
  return {
    value:
      differences.length === 0
        ? preview.client_timezone
        : translateApp("{{clientTimezone}} · differs from {{details}}", {
            clientTimezone: preview.client_timezone,
            details: differences.join(" and "),
          }),
    anomalous: differences.length > 0,
  };
}

export function requesterValue(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution === "verified" && preview.client_ip) {
    return preview.client_ip;
  }
  return preview.client_ip_attribution === "unverified"
    ? translateApp("Not verified")
    : translateApp("IP unavailable on this deployment");
}

function requestedAtValue(preview: AuthDevicePreview, nowMs: number): string {
  return `${formatDeviceLoginRelativeTime(preview.initiated_at, nowMs)} · ${formatTimestamp(preview.initiated_at)}`;
}

function ManualCodeModal({
  visible,
  value,
  errorMessage,
  isPending,
  coolingDown,
  colors,
  styles,
  onChange,
  onDismiss,
  onSubmit,
}: {
  visible: boolean;
  value: string;
  errorMessage: string | null;
  isPending: boolean;
  coolingDown: boolean;
  colors: ReturnType<typeof useTheme>["colors"];
  styles: ReturnType<typeof createDeviceLoginStyles>;
  onChange: (value: string) => void;
  onDismiss: () => void;
  onSubmit: () => void;
}) {
  const { t } = useAppLanguage();
  const inputRef = useRef<TextInput>(null);
  const insets = useSafeAreaInsets();
  const reducedMotion = useReducedMotion();
  const normalizedCode = normalizeAuthDeviceUserCode(value);

  useEffect(() => {
    if (!visible) return;
    const timeout = setTimeout(() => inputRef.current?.focus(), 180);
    return () => clearTimeout(timeout);
  }, [visible]);

  return (
    <Modal
      visible={visible}
      transparent
      animationType={reducedMotion ? "none" : "slide"}
      statusBarTranslucent
      onRequestClose={() => {
        // Native Modal owns Android Back separately from BackHandler.
        if (Platform.OS === "android" && Keyboard.isVisible()) {
          Keyboard.dismiss();
          return;
        }
        onDismiss();
      }}
    >
      <KeyboardViewport>
      <View
        accessibilityViewIsModal
        style={[styles.modalRoot, { paddingTop: insets.top }]}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={translateApp("Close manual code entry")}
          style={styles.modalBackdrop}
          onPress={onDismiss}
        />
        <ScrollView
          style={styles.modalScroll}
          contentContainerStyle={[
            styles.modalCard,
            { paddingBottom: Math.max(insets.bottom, 16) },
          ]}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode={Platform.OS === "ios" ? "interactive" : "none"}
          automaticallyAdjustKeyboardInsets={false}
        >
          <View style={styles.modalHeader}>
            <View style={styles.modalHeaderCopy}>
              <Text style={styles.modalTitle}>
                {translateApp("Enter login code")}
              </Text>
              <Text style={styles.modalDescription}>
                {t(
                  "Enter a device login or Agent Key login code from the requesting device.",
                )}
              </Text>
            </View>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={translateApp("Close manual code entry")}
              disabled={isPending}
              onPress={onDismiss}
              style={({ pressed }) => [
                styles.modalCloseButton,
                pressed && styles.pressed,
                isPending && styles.disabled,
              ]}
            >
              <X size={20} color={colors.textSecondary} />
            </Pressable>
          </View>

          <View style={styles.modalField}>
            <Text style={styles.inputLabel}>{translateApp("Login code")}</Text>
            <AuthTextInput
              ref={inputRef}
              invalid={!!errorMessage}
              accessibilityLabel={translateApp("Login code")}
              accessibilityHint={translateApp(
                "Eight-character code shown on the requesting device",
              )}
              value={value}
              onChangeText={onChange}
              onSubmitEditing={onSubmit}
              editable={!isPending}
              autoCapitalize="characters"
              autoCorrect={false}
              textContentType="oneTimeCode"
              maxLength={11}
              returnKeyType="done"
              selectionColor={colors.primary}
              placeholder={"ABCD-EFGH"}
              placeholderTextColor={colors.textTertiary}
              selectTextOnFocus
              style={[
                styles.codeInput,
                errorMessage ? styles.codeInputError : null,
              ]}
            />
            {errorMessage ? (
              <Text accessibilityRole="alert" style={styles.fieldErrorText}>
                {errorMessage}
              </Text>
            ) : null}
          </View>

          <AuthSubmitButton
            loading={isPending}
            label={
              isPending ? translateApp("Checking...") : translateApp("Continue")
            }
            disabled={isPending || coolingDown || !normalizedCode}
            onPress={onSubmit}
          />
        </ScrollView>
      </View>
      </KeyboardViewport>
    </Modal>
  );
}

export function DeviceLoginScreen({ navigation, route }: Props) {
  const { t } = useAppLanguage();
  const cooldown = useLoginPreviewCooldown();
  const handoffLoginRequest = useLoginRequestHandoff(navigation);
  const inWorkspaceSheet = Boolean(useWorkspaceSheet());
  const { colors } = useTheme();
  const styles = useMemo(() => createDeviceLoginStyles(colors), [colors]);
  const { isAuthenticated } = useAuthSession();
  const [userCode, setUserCode] = useState(() =>
    formatAuthDeviceUserCode(route.params?.user_code ?? ""),
  );
  const [confirmedCode, setConfirmedCode] = useState<string | null>(null);
  const [preview, setPreview] = useState<AuthDevicePreview | null>(null);
  const [terminal, setTerminal] = useState<TerminalState>(null);
  const stepUp = useMfaStepUp(
    `${confirmedCode}:${isAuthenticated}:${terminal}`,
  );
  const [isScanning, setIsScanning] = useState(
    () => route.params?.start_scanner === true && !route.params?.user_code,
  );
  const [manualEntryVisible, setManualEntryVisible] = useState(false);
  const [isPreviewing, setIsPreviewing] = useState(false);
  const [decisionPending, setDecisionPending] = useState<
    "approve" | "deny" | null
  >(null);
  const [errorMessage, setErrorMessage] = useLocalizedState<string | null>(null);
  const [clockMs, setClockMs] = useState(Date.now());
  const [deadlineMs, setDeadlineMs] = useState<number | null>(null);
  const currentDeadline = useRef<number | null>(null);
  const [rawUserAgentExpanded, setRawUserAgentExpanded] = useState(false);
  const lastActionAt = useRef(0);
  const generation = useRef(0);
  const [linkedPreview, setLinkedPreview] = useState<{ code: string; generation: number } | null>(null);
  const actionBusy = useRef(false);
  const decisionOperation = useRef<symbol | null>(null);
  const deferredRoute = useRef(false);
  const [routeRevision, setRouteRevision] = useState(0);
  const [decisionUncertain, setDecisionUncertain] = useState<
    "approve" | "deny" | null
  >(null);
  const uncertainDecision = useRef(decisionUncertain);
  function updateDecisionUncertain(value: typeof decisionUncertain) {
    uncertainDecision.current = value;
    setDecisionUncertain(value);
  }
  const currentCode = useRef(confirmedCode);
  currentCode.current = confirmedCode;
  useWorkspaceBack({
    active: inWorkspaceSheet && isScanning,
    defaultRole: "back",
    disabled: !!decisionPending || stepUp.busy,
    options: { headerShown: inWorkspaceSheet },
    onBack: () => setIsScanning(false),
  });
  const localTimezone = useMemo(() => {
    try {
      return (
        Intl.DateTimeFormat(appLocale()).resolvedOptions().timeZone || null
      );
    } catch {
      return null;
    }
    // The device's time zone does not depend on the display language.
  }, []);

  useLayoutEffect(() => {
    if (
      decisionOperation.current ||
      uncertainDecision.current ||
      (terminal && deferredRoute.current)
    ) {
      if (route.params?.user_code !== undefined) deferredRoute.current = true;
      return;
    }
    if (route.params?.user_code === undefined) {
      setLinkedPreview(null);
      if (route.params?.start_scanner === true) setIsScanning(true);
      return;
    }
    generation.current += 1;
    actionBusy.current = false;
    setDecisionPending(null);
    setIsPreviewing(false);
    setUserCode(formatAuthDeviceUserCode(route.params.user_code));
    setConfirmedCode(null);
    setPreview(null);
    setTerminal(null);
    setErrorMessage(null);
    setManualEntryVisible(false);
    setIsScanning(false);
    setDeadlineMs(null);
    currentDeadline.current = null;
    setRawUserAgentExpanded(false);
    updateDecisionUncertain(null);
    const handoff = takeLoginRequest(route.params.user_code, "device");
    if (handoff?.result.kind === "device") {
      lastActionAt.current = handoff.at;
      acceptPreview(handoff.code, handoff.result.preview);
      setLinkedPreview(null);
    } else {
      setLinkedPreview({ code: route.params.user_code, generation: generation.current });
    }
  }, [route.params?.start_scanner, route.params?.user_code, routeRevision]);
  useEffect(() => {
    // Commit the decision or verification prompt before applying the newest link.
    if (
      decisionOperation.current ||
      terminal ||
      decisionUncertain ||
      !deferredRoute.current
    )
      return;
    deferredRoute.current = false;
    setRouteRevision((revision) => revision + 1);
  }, [decisionPending, terminal, decisionUncertain]);

  function applyDeferredRoute() {
    if (!deferredRoute.current) return false;
    deferredRoute.current = false;
    const incoming = route.params?.user_code;
    if (
      incoming === undefined ||
      (terminal &&
        normalizeAuthDeviceUserCode(incoming) ===
          normalizeAuthDeviceUserCode(confirmedCode ?? userCode))
    )
      return false;
    updateDecisionUncertain(null);
    setRouteRevision((revision) => revision + 1);
    return true;
  }

  useEffect(
    () => () => {
      generation.current += 1;
      decisionOperation.current = null;
    },
    [],
  );

  function acceptPreview(code: string, result: AuthDevicePreview) {
    setConfirmedCode(code);
    refreshPreview(result);
    setRawUserAgentExpanded(false);
  }

  function refreshPreview(result: AuthDevicePreview) {
    updateDecisionUncertain(null);
    setErrorMessage(null);
    setPreview(result);
    const now = Date.now();
    const deadline = resolveDeviceLoginDeadlineMs(
      result.expires_at,
      result.seconds_remaining,
      now,
    );
    setDeadlineMs(deadline);
    currentDeadline.current = deadline;
    setClockMs(now);
    setTerminal(
      terminalFromPreview(result, decisionUncertain) ??
        (deadline <= now ? "expired" : null),
    );
  }

  function refreshRequest(automatic = false): number | Promise<void> {
    if (!confirmedCode || actionBusy.current) return 750;
    if (deadlineMs !== null && Date.now() >= deadlineMs && !decisionUncertain) {
      setTerminal("expired");
      return Promise.resolve();
    }
    const delay = Math.max(
      ACTION_THROTTLE_MS - (Date.now() - lastActionAt.current),
      automatic ? previewCooldownMs() : 0,
    );
    if (delay > 0) return delay;
    if (!automatic) uncertainPreview.reset();
    lastActionAt.current = Date.now();
    actionBusy.current = true;
    setIsPreviewing(true);
    const current = generation.current;
    const requestCode = confirmedCode;
    return previewLoginRequest(requestCode, "device")
      .then(
        (result) => {
          if (
            current === generation.current &&
            currentCode.current === requestCode
          )
            refreshPreview(result);
        },
        (error: unknown) => {
          if (current !== generation.current) return;
          setTerminal((previous) => terminalFromError(error) ?? previous);
          if (!automatic)
            setErrorMessage(
              () => (translateApp(deviceLoginErrorCopy(error), { host: loginServerHost() })),
            );
          cooldown.sync();
        },
      )
      .finally(() => {
        if (current === generation.current) {
          setIsPreviewing(false);
          actionBusy.current = false;
        }
      });
  }

  const uncertainPreview = useUncertainLoginPreview(
    Boolean(decisionUncertain && preview && !terminal && !decisionPending),
    `${generation.current}:${confirmedCode}`,
    () => refreshRequest(true),
  );
  useLoginRequestResume(Boolean(preview && !terminal), () => {
    if (actionBusy.current) return;
    const result = refreshRequest();
    if (typeof result === "number") return result;
  });

  useEffect(() => {
    if (!preview || terminal || deadlineMs === null) return;
    const interval = setInterval(() => {
      const now = Date.now();
      setClockMs(now);
      if (
        currentDeadline.current !== null &&
        now >= currentDeadline.current &&
        !decisionOperation.current &&
        !uncertainDecision.current
      ) {
        setTerminal((previous) => previous ?? "expired");
        clearInterval(interval);
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [deadlineMs, preview, terminal, decisionUncertain]);

  const claimAction = useCallback(() => {
    const now = Date.now();
    if (now - lastActionAt.current < ACTION_THROTTLE_MS) return false;
    lastActionAt.current = now;
    return true;
  }, []);

  const runPreview = useCallback(
    async (candidate: string, automatic = false): Promise<boolean> => {
      if (!automatic) setLinkedPreview(null);
      if (automatic ? previewCooldownMs() > 0 : cooldown.coolingDown) return false;
      if (actionBusy.current || !claimAction()) {
        setErrorMessage(() => (translateApp("Please wait a moment before trying again.")));
        return false;
      }
      actionBusy.current = true;
      setIsPreviewing(true);
      setErrorMessage(null);
      const current = generation.current;
      try {
        const result = await resolveLoginRequest(candidate, "device");
        if (current !== generation.current) return false;
        const normalized = normalizeAuthDeviceUserCode(candidate)!;
        if (result.kind === "agent-key") {
          if (handoffLoginRequest(normalized, result))
            navigation.navigate("AgentKeyLogin", { user_code: normalized });
        } else acceptPreview(normalized, result.preview);
        setManualEntryVisible(false);
        setIsScanning(false);
        return true;
      } catch (error) {
        if (current === generation.current) {
          const state = terminalFromError(error);
          if (state && state !== "unavailable") {
            setTerminal(state);
            setIsScanning(false);
            setManualEntryVisible(false);
          }
          setErrorMessage(
            () => (translateApp(deviceLoginErrorCopy(error), { host: loginServerHost() })),
          );
          cooldown.sync();
        }
        return false;
      } finally {
        if (current === generation.current) {
          actionBusy.current = false;
          setIsPreviewing(false);
        }
      }
    },
    [claimAction, cooldown.coolingDown, navigation, t],
  );

  // Incoming links use the same resolver, throttle and cooldown as manual
  // entry. Each route generation previews once; opening never makes a decision.
  useEffect(() => {
    if (!linkedPreview || linkedPreview.generation !== generation.current) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => {
      if (linkedPreview.generation !== generation.current) return;
      const delay = Math.max(
        ACTION_THROTTLE_MS - (Date.now() - lastActionAt.current),
        previewCooldownMs(),
        actionBusy.current ? ACTION_THROTTLE_MS : 0,
      );
      if (delay > 0) {
        timer = setTimeout(run, delay);
        return;
      }
      setLinkedPreview(null);
      void runPreview(linkedPreview.code, true);
    };
    run();
    return () => clearTimeout(timer);
  }, [linkedPreview, runPreview]);

  const handleScannedCode = (
    normalized: string,
    kind: "device" | "agent-key",
  ) => {
    if (kind === "agent-key") {
      setIsScanning(false);
      navigation.navigate("AgentKeyLogin", { user_code: normalized });
      return;
    }
    const formatted = formatAuthDeviceUserCode(normalized);
    setUserCode(formatted);
    setManualEntryVisible(false);
    setIsScanning(false);
    void runPreview(formatted);
  };

  const handleManualSubmit = async () => {
    const accepted = await runPreview(userCode);
    if (!accepted) return;
    setManualEntryVisible(false);
    setIsScanning(false);
  };

  const handleDecision = async (
    decision: "approve" | "deny",
    verifiedRetry = false,
  ) => {
    if (!confirmedCode || !preview || terminal || actionBusy.current) return;
    if (
      deadlineMs === null ||
      (deadlineMs <= Date.now() && !decisionUncertain)
    ) {
      setTerminal("expired");
      return;
    }
    if (!verifiedRetry && !claimAction()) {
      setErrorMessage(() => (translateApp("Please wait a moment before trying again.")));
      return;
    }

    uncertainPreview.reset();
    actionBusy.current = true;
    const owner = Symbol("login-decision");
    decisionOperation.current = owner;
    const current = generation.current;
    setDecisionPending(decision);
    setErrorMessage(null);
    let dispatched = false;
    try {
      if (decision === "approve") {
        dispatched = true;
        await mobileApi.approveAuthDevice(confirmedCode);
        if (current === generation.current) setTerminal("approved");
      } else {
        dispatched = true;
        await mobileApi.denyAuthDevice(confirmedCode);
        if (current === generation.current) setTerminal("denied");
      }
    } catch (error) {
      if (current !== generation.current) return;
      actionBusy.current = false;
      if (dispatched && isLoginDecisionUncertain(error))
        updateDecisionUncertain(decision);
      if (
        classifyLoginError(error) === "mfa_step_up" &&
        (await stepUp.handle(error, () => handleDecision(decision, true)))
      )
        return;
      setTerminal(
        (previous) => terminalFromError(error, decisionUncertain) ?? previous,
      );
      setErrorMessage(() => (translateApp(deviceLoginErrorCopy(error), { host: loginServerHost() })));
    } finally {
      if (decisionOperation.current === owner) {
        decisionOperation.current = null;
        actionBusy.current = false;
        setDecisionPending(null);
      }
    }
  };

  const handleBack = () => {
    if (decisionOperation.current) return;
    if ((terminal || uncertainDecision.current) && applyDeferredRoute()) return;
    if (navigation.canGoBack()) {
      navigation.goBack();
    } else if (isAuthenticated) {
      navigation.dispatch(workspaceNavigateAction("Activity"));
    } else {
      navigation.dispatch(StackActions.replace("Auth"));
    }
  };

  const handleRestart = () => {
    if (decisionOperation.current) return;
    if (applyDeferredRoute()) return;
    updateDecisionUncertain(null);
    generation.current += 1;
    actionBusy.current = false;
    setDecisionPending(null);
    setIsPreviewing(false);
    setTerminal(null);
    setPreview(null);
    setConfirmedCode(null);
    setUserCode("");
    setDeadlineMs(null);
    currentDeadline.current = null;
    setErrorMessage(null);
    setIsScanning(false);
    navigation.setParams({
      user_code: undefined,
      start_scanner: undefined,
    });
  };

  if (isScanning) {
    return (
      <>
        <DeviceCodeScanner
          showHeader={!inWorkspaceSheet}
          paused={manualEntryVisible || isPreviewing}
          onCancel={handleBack}
          onCode={handleScannedCode}
          onManualEntry={() => {
            if (!cooldown.coolingDown) setErrorMessage(null);
            setManualEntryVisible(true);
          }}
        />
        <ManualCodeModal
          visible={manualEntryVisible}
          value={userCode}
          errorMessage={errorMessage}
          isPending={isPreviewing}
          coolingDown={cooldown.coolingDown}
          colors={colors}
          styles={styles}
          onChange={(value) => {
            setUserCode(formatAuthDeviceUserCode(value));
            if (!cooldown.coolingDown) setErrorMessage(null);
          }}
          onDismiss={() => {
            setManualEntryVisible(false);
            if (!cooldown.coolingDown) setErrorMessage(null);
          }}
          onSubmit={() => void handleManualSubmit()}
        />
      </>
    );
  }

  const secondsRemaining =
    deadlineMs === null
      ? 0
      : secondsUntilDeviceLoginDeadline(deadlineMs, clockMs);
  const isExpired =
    Boolean(preview) &&
    secondsRemaining === 0 &&
    !decisionUncertain &&
    !decisionPending;
  const waitingForDecision = Boolean(
    decisionUncertain || (decisionPending && secondsRemaining === 0),
  );
  const isPending = isPreviewing || decisionPending !== null;
  const loginCode = formatAuthDeviceUserCode(confirmedCode ?? userCode);
  const originValue = preview
    ? formatDeviceLoginOriginValue(
        preview.initiating_origin_status,
        preview.initiating_origin,
      )
    : null;
  const reportedTimezone = preview
    ? timezoneRow(preview, localTimezone)
    : { value: translateApp("Not reported"), anomalous: false };
  const clientKindLabel = preview
    ? {
        cli: translateApp("CLI client"),
        browser: translateApp("Browser client"),
        mobile: translateApp("Mobile client"),
        unknown: translateApp("Not identified"),
      }[preview.client_kind]
    : translateApp("Not identified");
  const valueTones = resolveDeviceLoginValueTones(
    originValue !== null,
    reportedTimezone.anomalous,
    secondsRemaining,
  );

  if (terminal) {
    return (
      <LoginRequestTerminalView
        state={terminal}
        clientKind={preview?.client_kind}
        inSheet={inWorkspaceSheet}
        disabled={decisionPending !== null}
        onDone={handleBack}
        onRestart={handleRestart}
      />
    );
  }

  if (stepUp.prompt) {
    return (
      <ScreenContainer>
        <KeyboardAvoidingView
          style={styles.fill}
          behavior={Platform.OS === "ios" ? "padding" : undefined}
        >
          <ScrollView
            contentContainerStyle={[
              styles.content,
              !inWorkspaceSheet && styles.rootContent,
            ]}
            keyboardShouldPersistTaps="handled"
          >
            {stepUp.prompt}
          </ScrollView>
        </KeyboardAvoidingView>
      </ScreenContainer>
    );
  }
  return (
    <ScreenContainer>
      <KeyboardAvoidingView
        style={styles.fill}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
      >
        <ScrollView
          contentContainerStyle={[
            styles.content,
            !inWorkspaceSheet && styles.rootContent,
          ]}
          keyboardShouldPersistTaps="handled"
        >
          {!inWorkspaceSheet && (
            <View style={styles.screenHeader}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={translateApp("Back")}
                hitSlop={4}
                onPress={handleBack}
                style={({ pressed }) => [
                  styles.headerBackButton,
                  pressed && styles.pressed,
                ]}
              >
                <ChevronLeft size={24} color={colors.textPrimary} />
              </Pressable>
              <Text style={styles.title}>
                {translateApp("Approve device login")}
              </Text>
            </View>
          )}
          <Text style={styles.subtitle}>
            {t(
              preview
                ? "Review the request details."
                : "Enter a device login or Agent Key login code from the requesting device.",
            )}
          </Text>
          {!preview && (
            <PrimaryButton
              label={translateApp("Agent Key login")}
              kind="ghost"
              onPress={() => navigation.navigate("AgentKeyLogin")}
            />
          )}

          {!preview ? (
            <View style={styles.inputSection}>
              <Text style={styles.inputLabel}>
                {translateApp("Login code")}
              </Text>
              <AuthTextInput
                invalid={!!errorMessage}
                accessibilityLabel={translateApp("Login code")}
                value={userCode}
                onChangeText={(value) => {
                  setLinkedPreview(null);
                  setUserCode(formatAuthDeviceUserCode(value));
                  if (!cooldown.coolingDown) setErrorMessage(null);
                }}
                editable={!isPending}
                autoCapitalize="characters"
                autoCorrect={false}
                textContentType="oneTimeCode"
                maxLength={11}
                returnKeyType="done"
                onSubmitEditing={() => {
                  void runPreview(userCode);
                }}
                selectionColor={colors.primary}
                placeholder={"ABCD-EFGH"}
                placeholderTextColor={colors.textTertiary}
                style={[
                  styles.codeInput,
                  errorMessage ? styles.codeInputError : null,
                ]}
              />
              {errorMessage ? (
                <Text accessibilityRole="alert" style={styles.fieldErrorText}>
                  {errorMessage}
                </Text>
              ) : null}
              <AuthSubmitButton
                loading={isPreviewing}
                label={
                  isPreviewing
                    ? translateApp("Checking request...")
                    : translateApp("Continue")
                }
                onPress={() => void runPreview(userCode)}
                disabled={
                  isPending ||
                  cooldown.coolingDown ||
                  !normalizeAuthDeviceUserCode(userCode)
                }
              />
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={translateApp("Scan login QR code")}
                disabled={isPending}
                onPress={() => setIsScanning(true)}
                style={({ pressed }) => [
                  styles.scanButton,
                  pressed && styles.pressed,
                  isPending && styles.disabled,
                ]}
              >
                <ScanQrCode size={20} color={colors.primary} />
                <Text style={styles.scanButtonText}>
                  {translateApp("Scan QR code")}
                </Text>
              </Pressable>
            </View>
          ) : (
            <View style={styles.previewSection}>
              <View style={styles.detailPanel}>
                {loginCode ? (
                  <DetailRow
                    label={translateApp("Login code")}
                    value={loginCode}
                    mono
                    styles={styles}
                  />
                ) : null}
                {/*
                  A signal whose "good" state can be produced by an attacker
                  choosing what to send must never render as a positive assurance.
                  Origin is forgeable on this public endpoint, and even a first-party
                  proof would not stop a copied genuine QR, so only negative states
                  render here.
                */}
                {originValue ? (
                  <DetailRow
                    label={translateApp("Started from")}
                    value={originValue}
                    tone={valueTones.origin}
                    styles={styles}
                  />
                ) : null}
                <DetailRow
                  label={translateApp("Requester")}
                  value={requesterValue(preview)}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Location")}
                  value={formatLocation(preview)}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Network")}
                  value={formatNetwork(preview)}
                  styles={styles}
                />
                {preview.client_ip_attribution === "unverified" &&
                preview.client_ip ? (
                  <DetailRow
                    label={translateApp("Reported IP")}
                    value={translateApp("{{clientIp}} · unverified", {
                      clientIp: preview.client_ip,
                    })}
                    mono
                    styles={styles}
                  />
                ) : null}
                <DetailRow
                  label={translateApp("Requested")}
                  value={requestedAtValue(preview, clockMs)}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Reported device")}
                  value={formatDevice(preview)}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Reported client")}
                  value={preview.client_app ?? clientKindLabel}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Platform")}
                  value={preview.client_platform ?? translateApp("Not identified")
                  }
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Form factor")}
                  value={
                    preview.client_form_factor
                      ? `${preview.client_form_factor[0]?.toUpperCase() ?? ""}${preview.client_form_factor.slice(1)}`
                      : translateApp("Not reported")
                  }
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Timezone")}
                  value={reportedTimezone.value}
                  tone={valueTones.timezone}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Locale")}
                  value={preview.client_locale ?? translateApp("Not reported")}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Screen")}
                  value={formatScreen(preview)}
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Processor")}
                  value={
                    preview.client_hardware_concurrency === null
                      ? translateApp("Not reported")
                      : translateApp("{{clientHardwareConcurrency}} logical processors", {
                          clientHardwareConcurrency: preview.client_hardware_concurrency,
                        })
                  }
                  styles={styles}
                />
                <DetailRow
                  label={translateApp("Memory")}
                  value={
                    preview.client_device_memory === null
                      ? translateApp("Not reported")
                      : translateApp("{{gigabytes}} GB", {
                          gigabytes: preview.client_device_memory,
                        })
                  }
                  styles={styles}
                />
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={
                    rawUserAgentExpanded
                      ? translateApp("Hide raw user agent")
                      : translateApp("Show raw user agent")
                  }
                  onPress={() =>
                    setRawUserAgentExpanded((expanded) => !expanded)
                  }
                  style={({ pressed }) => [
                    styles.rawUserAgentButton,
                    pressed && styles.pressed,
                  ]}
                >
                  <Text style={styles.rawUserAgentLabel}>
                    {translateApp("Raw user agent")}
                  </Text>
                  {rawUserAgentExpanded ? (
                    <ChevronUp size={16} color={colors.textMuted} />
                  ) : (
                    <ChevronDown size={16} color={colors.textMuted} />
                  )}
                </Pressable>
                {rawUserAgentExpanded ? (
                  <Text style={styles.rawUserAgentValue}>
                    {preview.client_user_agent ?? translateApp("Not provided")}
                  </Text>
                ) : null}
                <DetailRow
                  label={translateApp("Expires in")}
                  value={
                    waitingForDecision
                      ? t("Waiting for NyxID…")
                      : isExpired
                        ? translateApp("Expired")
                        : formatRemaining(secondsRemaining)
                  }
                  tone={waitingForDecision ? "default" : valueTones.expiry}
                  styles={styles}
                />
              </View>

              <View style={styles.caution}>
                <Info
                  size={spacing.xl}
                  color={colors.textMuted}
                  style={styles.cautionIcon}
                />
                <Text style={styles.cautionText}><Translation message="Only approve if you started this sign-in. Confirm this matches the code shown on the requesting device or terminal. Reject if it does not match. {{warning}}" values={{ warning: <Text style={styles.cautionDanger}>
                    {translateApp("If anything looks unfamiliar, reject it.")}
                  </Text> }} />
                </Text>
              </View>

              {isAuthenticated ? (
                <View style={{ gap: spacing.md }}>
                  {preview.supports_grant_choice && (
                    <PrimaryButton
                      label={translateApp("Restricted Agent Key")}
                      kind="ghost"
                      disabled={isPending || isExpired}
                      onPress={() =>
                        navigation.navigate("AgentKeyLogin", {
                          user_code: confirmedCode ?? userCode,
                          flow: "device",
                        })
                      }
                    />
                  )}
                  <Text style={styles.cautionText}>
                    {translateApp(
                      "A full account session grants this machine your account, service, credential, and organization access. A restricted Agent Key grants only the permissions and resources you confirm next.",
                    )}
                  </Text>
                  <View style={styles.decisionButton}>
                    <PrimaryButton
                      label={
                        decisionPending === "deny" ? translateApp("Rejecting...")
                          : translateApp("Reject")
                      }
                      kind="danger"
                      disabled={isPending || isExpired}
                      onPress={() => void handleDecision("deny")}
                    />
                  </View>
                  <View style={styles.decisionButton}>
                    <AuthSubmitButton
                      loading={decisionPending === "approve"}
                      label={
                        decisionPending === "approve"
                          ? translateApp("Approving...")
                          : translateApp("Approve full account session")
                      }
                      disabled={isPending || isExpired}
                      onPress={() => void handleDecision("approve")}
                    />
                  </View>
                </View>
              ) : (
                <View style={styles.signInNotice}>
                  <Text style={styles.signInText}>
                    {translateApp("Sign in to approve or reject this request.")}
                  </Text>
                  <AuthSubmitButton
                    label={translateApp("Sign in")}
                    onPress={() => navigation.navigate("Auth")}
                  />
                </View>
              )}
            </View>
          )}

          {errorMessage && preview ? (
            <Text accessibilityRole="alert" style={styles.errorText}>
              {errorMessage}
            </Text>
          ) : null}
          {decisionUncertain ? (
            <View style={{ gap: spacing.md }}>
              <PrimaryButton
                label={t("Done")}
                kind="ghost"
                disabled={decisionPending !== null}
                onPress={handleBack}
              />
              <PrimaryButton
                label={t("Scan or enter another code")}
                kind="ghost"
                disabled={decisionPending !== null}
                onPress={handleRestart}
              />
            </View>
          ) : null}
          {isPending ? <ActivityIndicator color={colors.primary} /> : null}
        </ScrollView>
      </KeyboardAvoidingView>
    </ScreenContainer>
  );
}
