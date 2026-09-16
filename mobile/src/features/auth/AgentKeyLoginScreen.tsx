import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { ChevronLeft } from "lucide-react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type { RootStackParamList } from "../../app/AppNavigator";
import { ScreenContainer } from "../../components/ScreenContainer";
import { PrimaryButton } from "../../components/PrimaryButton";
import { useTheme } from "../../theme/ThemeContext";
import { agentKeyLoginApi } from "../../lib/api/agentKeyLoginApi";
import { loginCodeApi, type IssuedLoginCode, type LoginCodeGrant } from "../../lib/api/loginCodeApi";
import { LoginCodeStatusView } from "./LoginCodeStatusView";
import {
  agentKeyApproveSchema,
  type AgentKeyApprove,
  type AgentKeyOptions,
  type AgentKeyPreview,
  type AgentKeySummary,
} from "../../lib/api/agentKeyLoginSchema";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import {
  formatAuthDeviceUserCode,
  normalizeAuthDeviceUserCode,
  supportsRestrictedDeviceLogin,
} from "./deviceUserCode";
import {
  formatDeviceLoginOriginValue,
  formatDeviceLoginRelativeTime,
  resolveDeviceLoginDeadlineMs,
  secondsUntilDeviceLoginDeadline,
} from "./deviceLoginPreview";
import { useAuthSession } from "./AuthSessionContext";
import {
  DetailRow,
  formatLocation,
  formatNetwork,
  formatDevice,
  requesterValue,
} from "./DeviceLoginScreen";
import { AgentKeyCreateForm } from "./AgentKeyCreateForm";
import { DeviceCodeScanner } from "./DeviceCodeScanner";
import {
  agentKeyLoginError,
  issuanceNotice,
  newKeySummary,
  permissionRows,
} from "./agentKeyLoginModel";

type Props = NativeStackScreenProps<RootStackParamList, "AgentKeyLogin">;
type Step =
  | "code"
  | "review"
  | "options"
  | "new"
  | "confirm"
  | "account"
  | "approved"
  | "denied"
  | "expired";

export function AgentKeyLoginScreen({ navigation, route }: Props) {
  const flow = route.params?.flow ?? "agent-key";
  const mint = route.params?.mint ?? false;
  const [issued, setIssued] = useState<IssuedLoginCode | null>(null);
  const clearIssuedCode = useCallback(() => setIssued((value) => value?.code ? { ...value, code: "" } : value), []);
  const { colors } = useTheme();
  const styles = useMemo(() => createDeviceLoginStyles(colors), [colors]);
  const { isAuthenticated } = useAuthSession();
  const [code, setCode] = useState(() =>
    formatAuthDeviceUserCode(route.params?.user_code ?? ""),
  );
  const [step, setStep] = useState<Step>(mint ? "review" : "code");
  const [preview, setPreview] = useState<AgentKeyPreview | null>(null);
  const [options, setOptions] = useState<AgentKeyOptions | null>(null);
  const [selection, setSelection] = useState<
    AgentKeyApprove["selection"] | null
  >(null);
  const [summary, setSummary] = useState<AgentKeySummary | null>(null);
  const [credentialExpiry, setCredentialExpiry] = useState("");
  const [deadline, setDeadline] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now);
  const [pending, setPending] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lastAction = useRef(0);
  const generation = useRef(0);
  const previousMode = useRef({flow, mint});
  useEffect(() => {
    const modeChanged = previousMode.current.flow !== flow || previousMode.current.mint !== mint;
    previousMode.current = {flow, mint};
    if (route.params?.user_code === undefined && !modeChanged) return;
    generation.current += 1;
    setIssued(null);
    setCode(formatAuthDeviceUserCode(route.params?.user_code ?? ""));
    setStep(mint ? "review" : "code");
    setPreview(null);
    setOptions(null);
    setSummary(null);
    setSelection(null);
    setDeadline(null);
    setCredentialExpiry("");
    setError(null);
    setPending(false);
    return () => {
      generation.current += 1;
    };
  }, [route.params?.user_code, flow, mint]);
  useEffect(() => () => { generation.current += 1; }, []);
  const remaining =
    deadline === null ? null : secondsUntilDeviceLoginDeadline(deadline, now);
  const expired = remaining === 0;
  const finish = useCallback(
    (result: "approved" | "denied" | "expired") => {
      setStep(result);
      setCode("");
      setPreview(null);
      setOptions(null);
      setSelection(null);
      setSummary(null);
      setCredentialExpiry("");
      setDeadline(null);
      setError(null);
      navigation.setParams({ user_code: undefined });
    },
    [navigation],
  );
  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => {
      const now = Date.now();
      setNow(now);
      if (now >= deadline && !pending) finish("expired");
    }, 1000);
    return () => clearInterval(timer);
  }, [deadline, pending, finish]);
  async function act(action: () => Promise<void>) {
    if (pending || Date.now() - lastAction.current < 750) return;
    lastAction.current = Date.now();
    setPending(true);
    setError(null);
    const current = generation.current;
    try {
      await action();
    } catch (failure) {
      if (current === generation.current) setError(agentKeyLoginError(failure));
    } finally {
      if (current === generation.current) setPending(false);
    }
  }
  async function loadPreview() {
    const normalized = normalizeAuthDeviceUserCode(code);
    if (!normalized) return;
    const current = generation.current;
    const result = await agentKeyLoginApi.preview(normalized, flow);
    if (current !== generation.current) return;
    if (result.status !== "pending") {
      finish(result.status === "delivered" ? "approved" : result.status);
      return;
    }
    setCode(formatAuthDeviceUserCode(normalized));
    setPreview(result);
    setDeadline(
      resolveDeviceLoginDeadlineMs(result.expires_at, result.seconds_remaining),
    );
    setNow(Date.now());
    setStep("review");
  }
  async function loadOptions() {
    if (expired) return;
    const current = generation.current;
    const result = mint ? await loginCodeApi.options() : await agentKeyLoginApi.options(code, flow);
    if (current === generation.current) {
      setOptions(result);
      setStep("options");
    }
  }
  async function decide(accept: boolean) {
    if (expired) return;
    const current = generation.current;
    if (mint) {
      if (accept && selection) await createCode({auth_kind: "agent_key", selection,
        ...(credentialExpiry ? {credential_expires_at: credentialExpiry} : {})});
      else setStep("review");
      return;
    }
    if (accept && selection) {
      const parsed = agentKeyApproveSchema.parse({
        user_code: code,
        selection,
        credential_expires_at: credentialExpiry || undefined,
      });
      if (
        parsed.credential_expires_at &&
        summary?.expires_at &&
        Date.parse(parsed.credential_expires_at) >
          Date.parse(summary.expires_at)
      )
        throw new Error("Credential expiry cannot exceed key expiry.");
      await agentKeyLoginApi.approve(parsed, flow);
    } else if (!accept) await agentKeyLoginApi.deny(code, flow);
    else return;
    if (current === generation.current) finish(accept ? "approved" : "denied");
  }
  async function createCode(grant: LoginCodeGrant) {
    const current = generation.current;
    const result = await loginCodeApi.mint(grant);
    if (current === generation.current) setIssued(result);
  }
  const back = () =>
    navigation.canGoBack()
      ? navigation.goBack()
      : navigation.navigate(isAuthenticated ? "Activity" : "Auth");
  if (scanning)
    return (
      <DeviceCodeScanner
        onCancel={() => setScanning(false)}
        onManualEntry={() => setScanning(false)}
        onCode={(value, kind) => {
          setScanning(false);
          if (kind === "device")
            navigation.navigate("DeviceLogin", { user_code: value });
          else {
            setCode(formatAuthDeviceUserCode(value));
            setStep("code");
          }
        }}
      />
    );
  const permissions = (key: AgentKeySummary) => (
    <View>
      {permissionRows(key).map((row) => (
        <DetailRow
          key={row.label}
          label={row.label}
          value={row.value}
          tone={row.warning ? "warning" : "default"}
          styles={styles}
        />
      ))}
    </View>
  );
  const terminal =
    step === "approved" || step === "denied" || step === "expired";
  const origin = preview
    ? formatDeviceLoginOriginValue(
        preview.initiating_origin_status,
        preview.initiating_origin,
      )
    : null;
  return (
    <ScreenContainer>
      <KeyboardAvoidingView
        style={styles.fill}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
      >
        <ScrollView
          contentContainerStyle={[styles.content, { gap: 16 }]}
          keyboardShouldPersistTaps="handled"
        >
          <View style={styles.screenHeader}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Back"
              onPress={back}
              style={styles.headerBackButton}
            >
              <ChevronLeft size={24} color={colors.textPrimary} />
            </Pressable>
            <Text style={styles.title}>{mint ? "One-time login code" : "Agent Key login"}</Text>
          </View>
          {issued ? <LoginCodeStatusView issued={issued} onClearCode={clearIssuedCode} onNew={() => {
            setIssued(null); setStep("review"); setSelection(null); setSummary(null); setOptions(null); setCredentialExpiry("");
          }} /> : terminal ? (
            <View style={styles.inputSection}>
              <Text style={styles.terminalTitle}>
                {step === "approved"
                  ? "Approved - return to the requesting device"
                  : step === "denied"
                    ? "Login rejected"
                    : "Login request expired"}
              </Text>
              <PrimaryButton label="Done" onPress={back} />
            </View>
          ) : (
            <>
              {step === "code" && (
                <View style={styles.inputSection}>
                  <Text style={styles.inputLabel}>User code</Text>
                  <TextInput
                    accessibilityLabel="User code"
                    value={code}
                    editable={!pending}
                    autoCapitalize="characters"
                    autoCorrect={false}
                    maxLength={11}
                    onChangeText={(value) =>
                      setCode(formatAuthDeviceUserCode(value))
                    }
                    style={styles.codeInput}
                  />
                  <PrimaryButton
                    label="Continue"
                    disabled={pending || !normalizeAuthDeviceUserCode(code)}
                    onPress={() => void act(loadPreview)}
                  />
                  <PrimaryButton
                    label="Scan QR code"
                    kind="ghost"
                    disabled={pending}
                    onPress={() => setScanning(true)}
                  />
                </View>
              )}
              {preview && (
                <View style={styles.previewSection}>
                  <DetailRow
                    label="User code"
                    value={formatAuthDeviceUserCode(code)}
                    mono
                    styles={styles}
                  />
                  <Text style={styles.cautionText}>
                    Confirm this matches the code shown on the requesting device or terminal. Reject if it does not match.
                  </Text>
                  {origin && (
                    <DetailRow
                      label="Started from"
                      value={origin}
                      tone="warning"
                      styles={styles}
                    />
                  )}
                  {[
                    ["Requester", requesterValue(preview)],
                    ["IP attribution", preview.client_ip_attribution],
                    ["Reported IP", preview.client_ip ?? "Unavailable"],
                    ["Location", formatLocation(preview)],
                    ["Network", formatNetwork(preview)],
                    ["Reported device", formatDevice(preview)],
                    [
                      "Requested profile",
                      preview.requested_profile ?? "Not provided",
                    ],
                    [
                      "Reported client",
                      preview.client_app ?? preview.client_kind,
                    ],
                    ["Platform", preview.client_platform ?? "Not identified"],
                    [
                      "Requested",
                      `${formatDeviceLoginRelativeTime(preview.initiated_at, now)} - ${new Date(preview.initiated_at).toLocaleString()}`,
                    ],
                    [
                      "Expires",
                      `${new Date(preview.expires_at).toLocaleString()} (${remaining ?? 0}s)`,
                    ],
                  ].map(([label, value]) => (
                    <DetailRow
                      key={label}
                      label={label!}
                      value={value!}
                      styles={styles}
                    />
                  ))}
                  <Text style={styles.cautionText}>
                    Only approve a request you started. Reported device details
                    and a matching network do not prove who is requesting
                    access.
                  </Text>
                </View>
              )}
              {expired && (
                <Text accessibilityRole="alert" style={styles.fieldErrorText}>
                  This request has expired. Start Agent Key login again.
                </Text>
              )}
              {step === "review" &&
                (isAuthenticated && (mint || flow === "agent-key" || supportsRestrictedDeviceLogin(code)) ? (
                  <PrimaryButton
                    label="Choose an Agent Key"
                    disabled={pending || expired}
                    onPress={() => void act(loadOptions)}
                  />
                ) : !isAuthenticated ? (
                  <PrimaryButton
                    label="Sign in on this phone"
                    onPress={() => navigation.navigate("Auth")}
                  />
                ) : <PrimaryButton label="Review account login" onPress={() => navigation.navigate("DeviceLogin", {user_code: code})} />)}
              {mint && step === "review" && isAuthenticated && <>
                <Text style={styles.cautionText}>Anyone with this code can redeem the selected access once, within five minutes. Restricted Agent Key access is recommended.</Text>
                <PrimaryButton label="Full account session" kind="ghost" disabled={pending} onPress={() => setStep("account")} />
              </>}
              {step === "account" && <View style={styles.inputSection}>
                <Text style={styles.inputLabel}>Confirm full account access</Text>
                <Text style={styles.cautionText}>This code grants access to your account, services, credentials, and organization permissions. The session can refresh until it expires or you revoke it.</Text>
                <PrimaryButton label="Generate account login code" disabled={pending} onPress={() => void act(() => createCode({auth_kind: "account_session"}))} />
                <PrimaryButton label="Back" kind="ghost" disabled={pending} onPress={() => setStep("review")} />
              </View>}
              {step === "options" && options && (
                <View style={styles.inputSection}>
                  {options.keys.map((key) => (
                    <View key={key.id} style={styles.inputSection}>
                      {permissions(key)}
                      <PrimaryButton
                        label={`Choose ${key.name}`}
                        kind="ghost"
                        disabled={pending || expired}
                        onPress={() =>
                          void act(async () => {
                            setSelection({
                              kind: "existing",
                              api_key_id: key.id,
                            });
                            setSummary(key);
                            setStep("confirm");
                          })
                        }
                      />
                    </View>
                  ))}
                  <PrimaryButton
                    label="Create a new key"
                    disabled={pending || expired}
                    onPress={() => setStep("new")}
                  />
                </View>
              )}
              {step === "new" && options && (
                <AgentKeyCreateForm
                  options={options}
                  initialValues={
                    selection?.kind === "new" ? selection : undefined
                  }
                  disabled={pending || expired}
                  onReview={(input) =>
                    void act(async () => {
                      setSelection(input);
                      setSummary(newKeySummary(input, options));
                      setStep("confirm");
                    })
                  }
                />
              )}
              {step === "confirm" && summary && (
                <View style={styles.inputSection}>
                  <Text style={styles.inputLabel}>
                    Confirm effective permissions
                  </Text>
                  {permissions(summary)}
                  <Text style={styles.cautionText}>
                    {issuanceNotice(selection?.kind === "existing")}
                  </Text>
                  <Text style={styles.inputLabel}>
                    Credential expiry (optional)
                  </Text>
                  <TextInput
                    accessibilityLabel="Credential expiry"
                    placeholder="YYYY-MM-DDTHH:mm:ssZ"
                    placeholderTextColor={colors.textMuted}
                    value={credentialExpiry}
                    editable={!pending}
                    onChangeText={setCredentialExpiry}
                    style={[
                      styles.codeInput,
                      { textAlign: "left", fontSize: 14 },
                    ]}
                  />
                  <PrimaryButton
                    label={mint ? "Generate restricted login code" : "Approve"}
                    disabled={pending || expired}
                    onPress={() => void act(() => decide(true))}
                  />
                  <PrimaryButton
                    label="Back to keys"
                    kind="ghost"
                    disabled={pending}
                    onPress={() => setStep("options")}
                  />
                </View>
              )}
              {preview && isAuthenticated && (
                <PrimaryButton
                  label="Reject"
                  kind="danger"
                  disabled={pending || expired}
                  onPress={() => void act(() => decide(false))}
                />
              )}
            </>
          )}
          {error && (
            <Text accessibilityRole="alert" style={styles.fieldErrorText}>
              {error}
            </Text>
          )}
        </ScrollView>
      </KeyboardAvoidingView>
    </ScreenContainer>
  );
}
