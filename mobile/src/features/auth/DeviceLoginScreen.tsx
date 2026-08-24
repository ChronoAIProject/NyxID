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
  ChevronLeft,
  ScanQrCode,
  ShieldCheck,
  ShieldX,
  TriangleAlert,
  X,
} from "lucide-react-native";

import type { RootStackParamList } from "../../app/AppNavigator";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { mobileApi } from "../../lib/api/mobileApi";
import type { AuthDevicePreview } from "../../lib/api/authDeviceApi";
import { useTheme } from "../../theme/ThemeContext";
import { DeviceCodeScanner } from "./DeviceCodeScanner";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import {
  formatAuthDeviceUserCode,
  normalizeAuthDeviceUserCode,
} from "./deviceUserCode";
import { useAuthSession } from "./AuthSessionContext";

type Props = NativeStackScreenProps<RootStackParamList, "DeviceLogin">;
type TerminalState = "approved" | "denied" | null;

const ACTION_THROTTLE_MS = 750;

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
  styles,
}: {
  label: string;
  value: string;
  styles: ReturnType<typeof createDeviceLoginStyles>;
}) {
  return (
    <View style={styles.detailRow}>
      <Text style={styles.detailLabel}>{label}</Text>
      <Text style={styles.detailValue}>{value}</Text>
    </View>
  );
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
  const lastActionAt = useRef(0);

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
  }, [route.params?.start_scanner, route.params?.user_code]);

  useEffect(() => {
    if (!preview || terminal) return;
    const interval = setInterval(() => setClockMs(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [preview, terminal]);

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
        setClockMs(Date.now());
        return true;
      } catch (error) {
        setErrorMessage(resolveErrorMessage(error));
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

    const expiresAt = Date.parse(preview.expires_at);
    if (!Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
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
      setErrorMessage(resolveErrorMessage(error));
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

  const resetRequest = () => {
    setUserCode("");
    setConfirmedCode(null);
    setPreview(null);
    setTerminal(null);
    setErrorMessage(null);
    setClockMs(Date.now());
    setManualEntryVisible(false);
    setIsScanning(true);
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

  const expiresAtMs = preview ? Date.parse(preview.expires_at) : Number.NaN;
  const secondsRemaining = Number.isFinite(expiresAtMs)
    ? Math.max(0, Math.ceil((expiresAtMs - clockMs) / 1000))
    : 0;
  const isExpired = Boolean(preview) && secondsRemaining === 0;
  const isPending = isPreviewing || decisionPending !== null;

  if (terminal) {
    const approved = terminal === "approved";
    return (
      <ScreenContainer>
        <View style={styles.terminalScreen}>
          <View
            style={[
              styles.terminalIcon,
              approved ? styles.successIcon : styles.deniedIcon,
            ]}
          >
            {approved ? (
              <ShieldCheck size={36} color={colors.success} />
            ) : (
              <ShieldX size={36} color={colors.danger} />
            )}
          </View>
          <Text style={styles.terminalTitle}>
            {approved ? "Login approved" : "Request denied"}
          </Text>
          <Text style={styles.terminalBody}>
            {approved
              ? "The requesting device can now complete sign-in."
              : "The requesting device cannot use this login request."}
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
          <Text style={styles.subtitle}>
            Review where the request started before allowing another device to
            sign in.
          </Text>

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
              <View style={styles.warningBanner}>
                <TriangleAlert size={20} color={colors.warningTone.text} />
                <Text style={styles.warningText}>
                  Reject this request if you do not recognize the device, IP
                  address, or time.
                </Text>
              </View>

              <View style={styles.detailPanel}>
                <DetailRow
                  label="Reported device"
                  value={preview.client_label ?? "Not provided"}
                  styles={styles}
                />
                <DetailRow
                  label="Reported client"
                  value={preview.client_user_agent ?? "Not provided"}
                  styles={styles}
                />
                <DetailRow
                  label="Requester"
                  value={`${preview.client_ip ?? "Unknown IP"} at ${formatTimestamp(preview.initiated_at)}`}
                  styles={styles}
                />
                <DetailRow
                  label="Expires in"
                  value={
                    isExpired ? "Expired" : formatRemaining(secondsRemaining)
                  }
                  styles={styles}
                />
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

              <PrimaryButton
                label="Use another code"
                kind="ghost"
                disabled={isPending}
                onPress={resetRequest}
              />
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
