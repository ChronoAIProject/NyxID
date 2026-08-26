import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import {
  ChevronDown,
  ChevronLeft,
  ChevronUp,
  Info,
  ScanQrCode,
  X,
} from "lucide-react-native";

import type { RootStackParamList } from "../../app/AppNavigator";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { MagicKeyIllustration } from "../../components/icons/empty-state/MagicKeyIllustration";
import { RoadBarrierIllustration } from "../../components/icons/empty-state/RoadBarrierIllustration";
import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";
import { mobileApi } from "../../lib/api/mobileApi";
import type { AuthDevicePreview } from "../../lib/api/authDeviceApi";
import { spacing } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
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

type Props = NativeStackScreenProps<RootStackParamList, "DeviceLogin">;
type TerminalState = "approved" | "denied" | null;

const ACTION_THROTTLE_MS = 750;
const COMPLETION_MARK_SIZE = spacing.huge * 7;
// Design-selected so completion marks stay clear without overpowering the title.
const COMPLETION_MARK_OPACITY = 0.85;

function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? "Unknown time"
    : parsed.toLocaleString();
}

function formatRemaining(seconds: number): string {
  const safeSeconds = Math.max(0, seconds);
  const minutes = Math.floor(safeSeconds / 60);
  const remainder = safeSeconds % 60;
  return `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function DetailRow({
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

function formatLocation(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution !== "verified") return "Not available";
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
    ? `IP timezone: ${preview.client_ip_timezone}`
    : "Not available";
}

function formatNetwork(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution !== "verified") return "Not available";
  const relation =
    preview.network_relation ??
    (preview.same_ip_as_viewer === true
      ? "same_ip"
      : preview.same_ip_as_viewer === false
        ? "different_ip"
        : null);
  if (relation === "same_ip") return "Same IP as this phone";
  if (relation === "same_network") return "Same network as this phone";
  if (relation === "different_network") return "Different network";
  if (relation === "different_ip") return "Different IP";
  return "Not available";
}

function formatScreen(preview: AuthDevicePreview): string {
  if (
    preview.client_screen_width === null ||
    preview.client_screen_height === null
  ) {
    return "Not reported";
  }
  const ratio =
    preview.client_device_pixel_ratio === null
      ? ""
      : ` at ${preview.client_device_pixel_ratio}x`;
  return `${preview.client_screen_width} x ${preview.client_screen_height} CSS px${ratio}`;
}

function formatDevice(preview: AuthDevicePreview): string {
  if (preview.client_label && preview.client_model) {
    return `${preview.client_label} · ${preview.client_model}`;
  }
  return preview.client_label ?? preview.client_model ?? "Not provided";
}

function timezoneRow(
  preview: AuthDevicePreview,
  localTimezone: string | null,
): { value: string; anomalous: boolean } {
  if (!preview.client_timezone)
    return { value: "Not reported", anomalous: false };
  const differsFromPhone =
    compareDeviceLoginTimezones(preview.client_timezone, localTimezone) ===
    "different";
  const differences = [
    differsFromPhone ? "this phone" : null,
    preview.client_timezone_matches_ip === false ? "IP location" : null,
  ].filter((value): value is string => value !== null);
  return {
    value:
      differences.length === 0
        ? preview.client_timezone
        : `${preview.client_timezone} · differs from ${differences.join(" and ")}`,
    anomalous: differences.length > 0,
  };
}

function requesterValue(preview: AuthDevicePreview): string {
  if (preview.client_ip_attribution === "verified" && preview.client_ip) {
    return preview.client_ip;
  }
  return preview.client_ip_attribution === "unverified"
    ? "Not verified"
    : "IP unavailable on this deployment";
}

function requestedAtValue(preview: AuthDevicePreview, nowMs: number): string {
  return `${formatDeviceLoginRelativeTime(preview.initiated_at, nowMs)} · ${formatTimestamp(preview.initiated_at)}`;
}

function ManualCodeModal({
  visible,
  value,
  errorMessage,
  isPending,
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
  colors: ReturnType<typeof useTheme>["colors"];
  styles: ReturnType<typeof createDeviceLoginStyles>;
  onChange: (value: string) => void;
  onDismiss: () => void;
  onSubmit: () => void;
}) {
  const inputRef = useRef<TextInput>(null);
  const insets = useSafeAreaInsets();
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
      animationType="slide"
      statusBarTranslucent
      onRequestClose={onDismiss}
    >
      <KeyboardAvoidingView
        accessibilityViewIsModal
        style={styles.modalRoot}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Close manual code entry"
          style={styles.modalBackdrop}
          onPress={onDismiss}
        />
        <View
          style={[
            styles.modalCard,
            { paddingBottom: Math.max(insets.bottom, 16) },
          ]}
        >
          <View style={styles.modalHeader}>
            <View style={styles.modalHeaderCopy}>
              <Text style={styles.modalTitle}>Enter login code</Text>
              <Text style={styles.modalDescription}>
                Enter the eight-character code shown on the device requesting
                access.
              </Text>
            </View>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Close manual code entry"
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
            <Text style={styles.inputLabel}>Login code</Text>
            <TextInput
              ref={inputRef}
              accessibilityLabel="Login code"
              accessibilityHint="Eight-character code shown on the requesting device"
              value={value}
              onChangeText={onChange}
              onSubmitEditing={onSubmit}
              editable={!isPending}
              autoCapitalize="characters"
              autoCorrect={false}
              textContentType="oneTimeCode"
              maxLength={9}
              returnKeyType="done"
              selectionColor={colors.primary}
              placeholder="ABCD-EFGH"
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

          <PrimaryButton
            label={isPending ? "Checking..." : "Continue"}
            disabled={isPending || !normalizedCode}
            onPress={onSubmit}
          />
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

export function DeviceLoginScreen({ navigation, route }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createDeviceLoginStyles(colors), [colors]);
  const { isAuthenticated } = useAuthSession();
  const [userCode, setUserCode] = useState(() =>
    formatAuthDeviceUserCode(route.params?.user_code ?? ""),
  );
  const [confirmedCode, setConfirmedCode] = useState<string | null>(null);
  const [preview, setPreview] = useState<AuthDevicePreview | null>(null);
  const [terminal, setTerminal] = useState<TerminalState>(null);
  const [isScanning, setIsScanning] = useState(
    () => route.params?.start_scanner === true && !route.params?.user_code,
  );
  const [manualEntryVisible, setManualEntryVisible] = useState(false);
  const [isPreviewing, setIsPreviewing] = useState(false);
  const [decisionPending, setDecisionPending] = useState<
    "approve" | "deny" | null
  >(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [clockMs, setClockMs] = useState(Date.now());
  const [deadlineMs, setDeadlineMs] = useState<number | null>(null);
  const [rawUserAgentExpanded, setRawUserAgentExpanded] = useState(false);
  const lastActionAt = useRef(0);
  const localTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    if (route.params?.user_code === undefined) {
      if (route.params?.start_scanner === true) setIsScanning(true);
      return;
    }
    setUserCode(formatAuthDeviceUserCode(route.params.user_code));
    setConfirmedCode(null);
    setPreview(null);
    setTerminal(null);
    setErrorMessage(null);
    setManualEntryVisible(false);
    setIsScanning(false);
    setDeadlineMs(null);
    setRawUserAgentExpanded(false);
  }, [route.params?.start_scanner, route.params?.user_code]);

  useEffect(() => {
    if (!preview || terminal || deadlineMs === null) return;
    const interval = setInterval(() => {
      const now = Date.now();
      setClockMs(now);
      if (now >= deadlineMs) clearInterval(interval);
    }, 1000);
    return () => clearInterval(interval);
  }, [deadlineMs, preview, terminal]);

  const claimAction = useCallback(() => {
    const now = Date.now();
    if (now - lastActionAt.current < ACTION_THROTTLE_MS) return false;
    lastActionAt.current = now;
    return true;
  }, []);

  const runPreview = useCallback(
    async (candidate: string): Promise<boolean> => {
      if (!claimAction()) {
        setErrorMessage("Please wait a moment before trying again.");
        return false;
      }

      const normalized = normalizeAuthDeviceUserCode(candidate);
      if (!normalized) {
        setErrorMessage("Enter a valid eight-character login code.");
        return false;
      }

      setIsPreviewing(true);
      setErrorMessage(null);
      try {
        const result = await mobileApi.previewAuthDevice(normalized);
        if (result.status === "denied") {
          setTerminal("denied");
          return true;
        }
        if (result.status !== "pending") {
          setErrorMessage(
            result.status === "expired"
              ? "This login request has expired."
              : "This login request was already completed.",
          );
          return false;
        }

        setConfirmedCode(normalized);
        setPreview(result);
        const now = Date.now();
        setDeadlineMs(
          resolveDeviceLoginDeadlineMs(
            result.expires_at,
            result.seconds_remaining,
            now,
          ),
        );
        setClockMs(now);
        setRawUserAgentExpanded(false);
        return true;
      } catch (error) {
        setErrorMessage(resolveAuthDeviceErrorMessage(error));
        return false;
      } finally {
        setIsPreviewing(false);
      }
    },
    [claimAction],
  );

  const handleScannedCode = (normalized: string) => {
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

  const handleDecision = async (decision: "approve" | "deny") => {
    if (!confirmedCode || !preview) return;
    if (!claimAction()) {
      setErrorMessage("Please wait a moment before trying again.");
      return;
    }

    if (deadlineMs === null || deadlineMs <= Date.now()) {
      setErrorMessage("This login request has expired.");
      return;
    }

    setDecisionPending(decision);
    setErrorMessage(null);
    try {
      if (decision === "approve") {
        await mobileApi.approveAuthDevice(confirmedCode);
        setTerminal("approved");
      } else {
        await mobileApi.denyAuthDevice(confirmedCode);
        setTerminal("denied");
      }
    } catch (error) {
      setErrorMessage(resolveAuthDeviceErrorMessage(error));
    } finally {
      setDecisionPending(null);
    }
  };

  const handleBack = () => {
    if (navigation.canGoBack()) {
      navigation.goBack();
    } else {
      navigation.navigate(isAuthenticated ? "Activity" : "Auth");
    }
  };

  if (isScanning) {
    return (
      <>
        <DeviceCodeScanner
          paused={manualEntryVisible || isPreviewing}
          onCancel={handleBack}
          onCode={handleScannedCode}
          onManualEntry={() => {
            setErrorMessage(null);
            setManualEntryVisible(true);
          }}
        />
        <ManualCodeModal
          visible={manualEntryVisible}
          value={userCode}
          errorMessage={errorMessage}
          isPending={isPreviewing}
          colors={colors}
          styles={styles}
          onChange={(value) => {
            setUserCode(formatAuthDeviceUserCode(value));
            setErrorMessage(null);
          }}
          onDismiss={() => {
            setManualEntryVisible(false);
            setErrorMessage(null);
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
  const isExpired = Boolean(preview) && secondsRemaining === 0;
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
    : { value: "Not reported", anomalous: false };
  const clientKindLabel = preview
    ? {
        cli: "CLI client",
        browser: "Browser client",
        mobile: "Mobile client",
        unknown: "Not identified",
      }[preview.client_kind]
    : "Not identified";
  const valueTones = resolveDeviceLoginValueTones(
    originValue !== null,
    reportedTimezone.anomalous,
    secondsRemaining,
  );

  if (terminal) {
    const approved = terminal === "approved";
    return (
      <ScreenContainer>
        <View style={styles.terminalScreen}>
          {approved ? (
            <MagicKeyIllustration
              size={COMPLETION_MARK_SIZE}
              color={colors.success}
              opacity={COMPLETION_MARK_OPACITY}
            />
          ) : (
            <RoadBarrierIllustration
              size={COMPLETION_MARK_SIZE}
              color={colors.danger}
              opacity={COMPLETION_MARK_OPACITY}
            />
          )}
          <Text style={styles.terminalTitle}>
            {approved ? "Login approved" : "Request denied"}
          </Text>
          <PrimaryButton label="Done" onPress={handleBack} />
        </View>
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
          contentContainerStyle={styles.content}
          keyboardShouldPersistTaps="handled"
        >
          <View style={styles.screenHeader}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Back"
              hitSlop={4}
              onPress={handleBack}
              style={({ pressed }) => [
                styles.headerBackButton,
                pressed && styles.pressed,
              ]}
            >
              <ChevronLeft size={24} color={colors.textPrimary} />
            </Pressable>
            <Text style={styles.title}>Approve device login</Text>
          </View>
          <Text style={styles.subtitle}>Review the request details.</Text>

          {!preview ? (
            <View style={styles.inputSection}>
              <Text style={styles.inputLabel}>Login code</Text>
              <TextInput
                accessibilityLabel="Login code"
                value={userCode}
                onChangeText={(value) => {
                  setUserCode(formatAuthDeviceUserCode(value));
                  setErrorMessage(null);
                }}
                editable={!isPending}
                autoCapitalize="characters"
                autoCorrect={false}
                textContentType="oneTimeCode"
                maxLength={9}
                returnKeyType="done"
                onSubmitEditing={() => {
                  if (normalizeAuthDeviceUserCode(userCode))
                    void runPreview(userCode);
                }}
                selectionColor={colors.primary}
                placeholder="ABCD-EFGH"
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
              <PrimaryButton
                label={isPreviewing ? "Checking request..." : "Continue"}
                onPress={() => void runPreview(userCode)}
                disabled={isPending || !normalizeAuthDeviceUserCode(userCode)}
              />
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Scan login QR code"
                disabled={isPending}
                onPress={() => setIsScanning(true)}
                style={({ pressed }) => [
                  styles.scanButton,
                  pressed && styles.pressed,
                  isPending && styles.disabled,
                ]}
              >
                <ScanQrCode size={20} color={colors.primary} />
                <Text style={styles.scanButtonText}>Scan QR code</Text>
              </Pressable>
            </View>
          ) : (
            <View style={styles.previewSection}>
              <View style={styles.detailPanel}>
                {loginCode ? (
                  <DetailRow
                    label="Login code"
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
                    label="Started from"
                    value={originValue}
                    tone={valueTones.origin}
                    styles={styles}
                  />
                ) : null}
                <DetailRow
                  label="Requester"
                  value={requesterValue(preview)}
                  styles={styles}
                />
                <DetailRow
                  label="Location"
                  value={formatLocation(preview)}
                  styles={styles}
                />
                <DetailRow
                  label="Network"
                  value={formatNetwork(preview)}
                  styles={styles}
                />
                {preview.client_ip_attribution === "unverified" &&
                preview.client_ip ? (
                  <DetailRow
                    label="Reported IP"
                    value={`${preview.client_ip} · unverified`}
                    mono
                    styles={styles}
                  />
                ) : null}
                <DetailRow
                  label="Requested"
                  value={requestedAtValue(preview, clockMs)}
                  styles={styles}
                />
                <DetailRow
                  label="Reported device"
                  value={formatDevice(preview)}
                  styles={styles}
                />
                <DetailRow
                  label="Reported client"
                  value={preview.client_app ?? clientKindLabel}
                  styles={styles}
                />
                <DetailRow
                  label="Platform"
                  value={preview.client_platform ?? "Not identified"}
                  styles={styles}
                />
                <DetailRow
                  label="Form factor"
                  value={
                    preview.client_form_factor
                      ? `${preview.client_form_factor[0]?.toUpperCase() ?? ""}${preview.client_form_factor.slice(1)}`
                      : "Not reported"
                  }
                  styles={styles}
                />
                <DetailRow
                  label="Timezone"
                  value={reportedTimezone.value}
                  tone={valueTones.timezone}
                  styles={styles}
                />
                <DetailRow
                  label="Locale"
                  value={preview.client_locale ?? "Not reported"}
                  styles={styles}
                />
                <DetailRow
                  label="Screen"
                  value={formatScreen(preview)}
                  styles={styles}
                />
                <DetailRow
                  label="Processor"
                  value={
                    preview.client_hardware_concurrency === null
                      ? "Not reported"
                      : `${preview.client_hardware_concurrency} logical processors`
                  }
                  styles={styles}
                />
                <DetailRow
                  label="Memory"
                  value={
                    preview.client_device_memory === null
                      ? "Not reported"
                      : `${preview.client_device_memory} GB`
                  }
                  styles={styles}
                />
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={
                    rawUserAgentExpanded
                      ? "Hide raw user agent"
                      : "Show raw user agent"
                  }
                  onPress={() =>
                    setRawUserAgentExpanded((expanded) => !expanded)
                  }
                  style={({ pressed }) => [
                    styles.rawUserAgentButton,
                    pressed && styles.pressed,
                  ]}
                >
                  <Text style={styles.rawUserAgentLabel}>Raw user agent</Text>
                  {rawUserAgentExpanded ? (
                    <ChevronUp size={16} color={colors.textMuted} />
                  ) : (
                    <ChevronDown size={16} color={colors.textMuted} />
                  )}
                </Pressable>
                {rawUserAgentExpanded ? (
                  <Text style={styles.rawUserAgentValue}>
                    {preview.client_user_agent ?? "Not provided"}
                  </Text>
                ) : null}
                <DetailRow
                  label="Expires in"
                  value={
                    isExpired ? "Expired" : formatRemaining(secondsRemaining)
                  }
                  tone={valueTones.expiry}
                  styles={styles}
                />
              </View>

              <View style={styles.caution}>
                <Info
                  size={spacing.xl}
                  color={colors.textMuted}
                  style={styles.cautionIcon}
                />
                <Text style={styles.cautionText}>
                  {"Only approve if you started this sign-in. "}
                  <Text style={styles.cautionDanger}>
                    If anything looks unfamiliar, reject it.
                  </Text>
                </Text>
              </View>

              {isAuthenticated ? (
                <View style={styles.decisionRow}>
                  <View style={styles.decisionButton}>
                    <PrimaryButton
                      label={
                        decisionPending === "deny" ? "Rejecting..." : "Reject"
                      }
                      kind="danger"
                      disabled={isPending || isExpired}
                      onPress={() => void handleDecision("deny")}
                    />
                  </View>
                  <View style={styles.decisionButton}>
                    <PrimaryButton
                      label={
                        decisionPending === "approve"
                          ? "Approving..."
                          : "Approve"
                      }
                      disabled={isPending || isExpired}
                      onPress={() => void handleDecision("approve")}
                    />
                  </View>
                </View>
              ) : (
                <View style={styles.signInNotice}>
                  <Text style={styles.signInText}>
                    Sign in to approve or reject this request.
                  </Text>
                  <PrimaryButton
                    label="Sign in"
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
          {isPending ? <ActivityIndicator color={colors.primary} /> : null}
        </ScrollView>
      </KeyboardAvoidingView>
    </ScreenContainer>
  );
}
