import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
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
  LoginRequestError,
  loginErrorCopy,
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
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
} from "react-native";
import { ChevronLeft, KeyRound, UserRound } from "lucide-react-native";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { workspaceNavigateAction } from "../../app/navigationHierarchy";
import { ScreenContainer } from "../../components/ScreenContainer";
import { PrimaryButton } from "../../components/PrimaryButton";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { useAuthTheme as useTheme } from "./useAuthTheme";
import { AuthTextInput } from "./AuthTextInput";
import { agentKeyLoginApi } from "../../lib/api/agentKeyLoginApi";
import type { AuthDevicePreview } from "../../lib/api/authDeviceSchema";
import {
  loginCodeApi,
  type IssuedLoginCode,
  type LoginCodeGrant,
} from "../../lib/api/loginCodeApi";
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
  agentKeyActionErrorCopy,
  issuanceNotice,
  newKeySummary,
  permissionRows,
} from "./agentKeyLoginModel";

type Props = SheetScreenProps<"AgentKeyLogin">;
type Step =
  | "code"
  | "review"
  | "options"
  | "new"
  | "confirm"
  | "account"
  | "approved"
  | "denied"
  | "expired"
  | "completed"
  | "unavailable";

function isTerminalStep(step: Step): step is LoginTerminal {
  return ["approved", "denied", "expired", "completed", "unavailable"].includes(
    step,
  );
}

export function AgentKeyLoginScreen({ navigation, route }: Props) {
  const { t } = useAppLanguage();
  const cooldown = useLoginPreviewCooldown();
  const handoffLoginRequest = useLoginRequestHandoff(navigation);
  const inWorkspaceSheet = Boolean(useWorkspaceSheet());
  const flow = route.params?.flow ?? "agent-key";
  const mint = route.params?.mint ?? false;
  const [issued, setIssued] = useState<IssuedLoginCode | null>(null);
  const clearIssuedCode = useCallback(
    () => setIssued((value) => (value?.code ? { ...value, code: "" } : value)),
    [],
  );
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
  const [writing, setWriting] = useState(false);
  const [scanning, setScanning] = useState(false);
  const stepUp = useMfaStepUp(
    `${flow}:${mint}:${code}:${isAuthenticated}:${step}`,
  );
  useWorkspaceBack({
    active: inWorkspaceSheet && scanning,
    disabled: writing || stepUp.busy,
    defaultRole: "back",
    options: { headerShown: inWorkspaceSheet },
    label: translateApp("Back to login"),
    onBack: () => setScanning(false),
  });
  const [error, setError] = useLocalizedState<string | null>(null);
  const lastAction = useRef(0);
  const actionBusy = useRef(false);
  const operation = useRef<{
    criticalWrite: boolean;
    dispatched: boolean;
    decision: "approve" | "deny" | null;
  } | null>(null);
  const [decisionUncertain, setDecisionUncertain] = useState<
    "approve" | "deny" | null
  >(null);
  const finishedRequestCode = useRef<string | null>(null);
  const currentCode = useRef(code);
  currentCode.current = code;
  const generation = useRef(0);
  const previousMode = useRef({ flow, mint });
  const deferredRoute = useRef(false);
  const [routeRevision, setRouteRevision] = useState(0);
  const latestRouteCode = useRef(route.params?.user_code);
  latestRouteCode.current = route.params?.user_code;
  useLayoutEffect(() => {
    if (
      operation.current?.criticalWrite ||
      decisionUncertain ||
      (isTerminalStep(step) && deferredRoute.current)
    ) {
      if (route.params?.user_code !== undefined) deferredRoute.current = true;
      return;
    }
    const modeChanged =
      previousMode.current.flow !== flow || previousMode.current.mint !== mint;
    previousMode.current = { flow, mint };
    if (route.params?.user_code === undefined && !modeChanged) return;
    generation.current += 1;
    actionBusy.current = false;
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
    setWriting(false);
    setDecisionUncertain(null);
    if (!mint && route.params?.user_code) {
      const handoff = takeLoginRequest(route.params.user_code, flow);
      if (handoff) {
        lastAction.current = handoff.at;
        acceptPreview(handoff.code, handoff.result.preview);
      }
    }
  }, [route.params?.user_code, flow, mint, routeRevision]);
  useEffect(() => {
    // Commit the decision or verification prompt before applying the newest link.
    if (
      operation.current ||
      isTerminalStep(step) ||
      decisionUncertain ||
      !deferredRoute.current
    )
      return;
    deferredRoute.current = false;
    setRouteRevision((revision) => revision + 1);
  }, [pending, writing, step, decisionUncertain]);
  function applyDeferredRoute() {
    if (!deferredRoute.current) return false;
    deferredRoute.current = false;
    const incoming = route.params?.user_code;
    if (
      incoming === undefined ||
      (isTerminalStep(step) &&
        normalizeAuthDeviceUserCode(incoming) === finishedRequestCode.current)
    )
      return false;
    setDecisionUncertain(null);
    setRouteRevision((revision) => revision + 1);
    return true;
  }
  useEffect(
    () => () => {
      generation.current += 1;
      operation.current = null;
    },
    [],
  );
  const remaining =
    deadline === null ? null : secondsUntilDeviceLoginDeadline(deadline, now);
  const expired = remaining === 0 && !decisionUncertain && !writing;
  const waitingForDecision = Boolean(
    decisionUncertain || (writing && remaining === 0),
  );
  const finish = useCallback(
    (result: LoginTerminal, requestCode = code) => {
      setStep(result);
      setCode("");
      setPreview(null);
      setOptions(null);
      setSelection(null);
      setSummary(null);
      setCredentialExpiry("");
      setDeadline(null);
      setError(null);
      const finishedCode = normalizeAuthDeviceUserCode(requestCode);
      finishedRequestCode.current = finishedCode;
      if (
        finishedCode &&
        normalizeAuthDeviceUserCode(latestRouteCode.current ?? "") ===
          finishedCode
      ) {
        deferredRoute.current = false;
        navigation.setParams({ user_code: undefined });
      }
    },
    [navigation, code],
  );
  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => {
      const now = Date.now();
      setNow(now);
      if (
        now >= deadline &&
        !pending &&
        !actionBusy.current &&
        !decisionUncertain
      )
        finish("expired");
    }, 1000);
    return () => clearInterval(timer);
  }, [deadline, pending, decisionUncertain, finish]);
  async function act(
    action: () => Promise<void>,
    verifiedRetry = false,
    criticalWrite = false,
    automatic = false,
  ) {
    if (actionBusy.current || pending) return;
    if (step === "code" && cooldown.coolingDown) return;
    if (
      !mint &&
      preview &&
      deadline !== null &&
      deadline <= Date.now() &&
      !decisionUncertain
    ) {
      finish("expired");
      return;
    }
    if (!verifiedRetry && Date.now() - lastAction.current < 750) {
      setError(() => (translateApp("Please wait a moment before trying again.")));
      return;
    }
    if (!automatic) uncertainPreview.reset();
    lastAction.current = Date.now();
    actionBusy.current = true;
    const owner: NonNullable<typeof operation.current> = {
      criticalWrite,
      dispatched: false,
      decision: null,
    };
    operation.current = owner;
    setPending(true);
    setWriting(criticalWrite);
    if (!automatic) setError(null);
    const current = generation.current;
    try {
      await action();
    } catch (failure) {
      if (current !== generation.current) return;
      actionBusy.current = false;
      if (
        !mint &&
        criticalWrite &&
        owner.decision !== null &&
        owner.dispatched &&
        isLoginDecisionUncertain(failure)
      )
        setDecisionUncertain(owner.decision);
      if (
        classifyLoginError(failure) === "mfa_step_up" &&
        (await stepUp.handle(failure, () => act(action, true, criticalWrite)))
      )
        return;
      const terminal = terminalFromError(failure, decisionUncertain);
      if (!mint && terminal && (step !== "code" || terminal !== "unavailable"))
        finish(terminal);
      else if (!automatic)
        setError(
          () => (translateApp(agentKeyActionErrorCopy(failure, step), {
            host: loginServerHost(),
          })),
        );
      cooldown.sync();
    } finally {
      if (operation.current === owner) {
        operation.current = null;
        actionBusy.current = false;
        setPending(false);
        setWriting(false);
      }
    }
  }
  async function loadPreview() {
    const normalized = normalizeAuthDeviceUserCode(code);
    if (!normalized) throw new LoginRequestError("malformed");
    const current = generation.current;
    // Explicit device-grant selection stays in this screen; generic Agent Key
    // entry resolves either code type before handing off the fetched preview.
    const result =
      flow === "device"
        ? {
            kind: "device" as const,
            preview: await previewLoginRequest(normalized, flow, "agent-key"),
          }
        : await resolveLoginRequest(normalized, "agent-key");
    if (current !== generation.current) return;
    if (flow !== "device" && result.kind === "device") {
      if (handoffLoginRequest(normalized, result))
        navigation.navigate("DeviceLogin", { user_code: normalized });
    } else acceptPreview(normalized, result.preview);
  }
  function acceptPreview(code: string, result: AuthDevicePreview) {
    if (!refreshPreview(result, code)) return;
    setCode(formatAuthDeviceUserCode(code));
    setStep("review");
  }
  function refreshPreview(result: AuthDevicePreview, requestCode = code) {
    setDecisionUncertain(null);
    setError(null);
    const terminal = terminalFromPreview(result, decisionUncertain);
    if (terminal) {
      finish(terminal, requestCode);
      return false;
    }
    const deadline = resolveDeviceLoginDeadlineMs(
      result.expires_at,
      result.seconds_remaining,
      Date.now(),
    );
    if (deadline <= Date.now()) {
      finish("expired", requestCode);
      return false;
    }
    setPreview({ requested_profile: null, interval: 5, ...result });
    setDeadline(deadline);
    setNow(Date.now());
    return true;
  }
  function refreshRequest(automatic = false): number | Promise<void> {
    if (actionBusy.current) return 750;
    const delay = Math.max(
      750 - (Date.now() - lastAction.current),
      automatic ? previewCooldownMs() : 0,
    );
    if (delay > 0) return delay;
    return act(
      async () => {
        const current = generation.current;
        const requestCode = code;
        const result = await previewLoginRequest(requestCode, flow, "agent-key");
        if (
          current === generation.current &&
          currentCode.current === requestCode
        )
          refreshPreview(result);
      },
      false,
      false,
      automatic,
    );
  }
  const uncertainPreview = useUncertainLoginPreview(
    Boolean(
      decisionUncertain &&
        preview &&
        !mint &&
        !isTerminalStep(step) &&
        !writing,
    ),
    `${generation.current}:${code}`,
    () => refreshRequest(true),
  );
  useLoginRequestResume(Boolean(preview && !mint), () => {
    if (actionBusy.current) return;
    if (deadline !== null && deadline <= Date.now() && !decisionUncertain) {
      finish("expired");
      return;
    }
    const result = refreshRequest();
    if (typeof result === "number") return result;
  });

  async function loadOptions() {
    if (expired) {
      finish("expired");
      return;
    }
    const current = generation.current;
    const result = mint
      ? await loginCodeApi.options()
      : await agentKeyLoginApi.options(code, flow);
    if (current === generation.current) {
      setOptions(result);
      setStep("options");
    }
  }
  async function decide(accept: boolean) {
    if (expired) {
      finish("expired");
      return;
    }
    const current = generation.current;
    if (mint) {
      if (accept && selection)
        await createCode({
          auth_kind: "agent_key",
          selection,
          ...(credentialExpiry
            ? { credential_expires_at: credentialExpiry }
            : {}),
        });
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
        throw localizedError("Credential expiry cannot exceed key expiry.");
      if (operation.current) {
        operation.current.decision = "approve";
        operation.current.dispatched = true;
      }
      await agentKeyLoginApi.approve(parsed, flow);
    } else if (!accept) {
      if (operation.current) {
        operation.current.decision = "deny";
        operation.current.dispatched = true;
      }
      await agentKeyLoginApi.deny(code, flow);
    } else return;
    if (current === generation.current) finish(accept ? "approved" : "denied");
  }
  async function createCode(grant: LoginCodeGrant) {
    const current = generation.current;
    if (operation.current) operation.current.dispatched = true;
    const result = await loginCodeApi.mint(grant);
    if (current === generation.current) setIssued(result);
  }
  const back = () => {
    if (writing || operation.current?.criticalWrite) return;
    if ((isTerminalStep(step) || decisionUncertain) && applyDeferredRoute())
      return;
    if (navigation.canGoBack()) {
      navigation.goBack();
    } else if (isAuthenticated) {
      navigation.dispatch(workspaceNavigateAction("Activity"));
    } else {
      navigation.navigate("Auth");
    }
  };
  const restart = () => {
    if (writing || operation.current?.criticalWrite) return;
    if (applyDeferredRoute()) return;
    setDecisionUncertain(null);
    generation.current += 1;
    actionBusy.current = false;
    setPending(false);
    setWriting(false);
    setStep("code");
    setCode("");
    setPreview(null);
    setDeadline(null);
    setError(null);
    setOptions(null);
    setSelection(null);
    setSummary(null);
    setCredentialExpiry("");
    navigation.setParams({ user_code: undefined, flow: undefined });
  };

  if (scanning)
    return (
      <DeviceCodeScanner
        showHeader={!inWorkspaceSheet}
        onCancel={() => setScanning(false)}
        onManualEntry={() => setScanning(false)}
        onCode={(value, kind) => {
          setScanning(false);
          if (kind === "staging-device")
            navigation.navigate("DeviceLogin", { user_code: value, staging_scan: true });
          else if (kind === "device")
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
  const terminal = isTerminalStep(step) ? step : null;
  const origin = preview
    ? formatDeviceLoginOriginValue(
        preview.initiating_origin_status,
        preview.initiating_origin,
      )
    : null;
  if (terminal)
    return (
      <LoginRequestTerminalView
        state={terminal}
        inSheet={inWorkspaceSheet}
        disabled={writing || pending}
        onDone={back}
        onRestart={restart}
      />
    );
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
            { gap: 16 },
          ]}
          keyboardShouldPersistTaps="handled"
        >
          {!inWorkspaceSheet && (
            <View style={styles.screenHeader}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={translateApp("Back")}
                onPress={back}
                style={styles.headerBackButton}
              >
                <ChevronLeft size={24} color={colors.textPrimary} />
              </Pressable>
              <Text style={styles.title}>
                {mint ? translateApp("One-time login code")
                  : translateApp("Agent Key login")}
              </Text>
            </View>
          )}
          {issued ? (
            <LoginCodeStatusView
              issued={issued}
              onClearCode={clearIssuedCode}
              onNew={() => {
                setIssued(null);
                setStep("review");
                setSelection(null);
                setSummary(null);
                setOptions(null);
                setCredentialExpiry("");
              }}
            />
          ) : (
            <>
              {step === "code" && (
                <View style={styles.inputSection}>
                  <Text style={styles.inputLabel}>
                    {t(
                      "Enter a device login or Agent Key login code from the requesting device.",
                    )}
                  </Text>
                  <AuthTextInput
                    accessibilityLabel={translateApp("User code")}
                    value={code}
                    editable={!pending}
                    autoCapitalize="characters"
                    autoCorrect={false}
                    maxLength={11}
                    returnKeyType="done"
                    onSubmitEditing={() => void act(loadPreview)}
                    onChangeText={(value) =>
                      setCode(formatAuthDeviceUserCode(value))
                    }
                    style={styles.codeInput}
                  />
                  <AuthSubmitButton
                    loading={pending}
                    label={translateApp("Continue")}
                    disabled={
                      pending ||
                      cooldown.coolingDown ||
                      !normalizeAuthDeviceUserCode(code)
                    }
                    onPress={() => void act(loadPreview)}
                  />
                  <PrimaryButton
                    label={translateApp("Scan QR code")}
                    kind="ghost"
                    disabled={pending}
                    onPress={() => setScanning(true)}
                  />
                </View>
              )}
              {preview && (
                <View style={styles.previewSection}>
                  <DetailRow
                    label={translateApp("User code")}
                    value={formatAuthDeviceUserCode(code)}
                    mono
                    styles={styles}
                  />
                  <Text style={styles.cautionText}>
                    {translateApp(
                      "Confirm this matches the code shown on the requesting device or terminal. Reject if it does not match.",
                    )}
                  </Text>
                  {origin && (
                    <DetailRow
                      label={translateApp("Started from")}
                      value={origin}
                      tone="warning"
                      styles={styles}
                    />
                  )}
                  {[
                    [translateApp("Requester"), requesterValue(preview)],
                    [
                      translateApp("IP attribution"), preview.client_ip_attribution],
                    [
                      translateApp("Reported IP"), preview.client_ip ?? translateApp("Unavailable"),
                    ],
                    [translateApp("Location"), formatLocation(preview)],
                    [translateApp("Network"), formatNetwork(preview)],
                    [translateApp("Reported device"), formatDevice(preview)],
                    [
                      translateApp("Requested profile"),
                      preview.requested_profile ?? translateApp("Not provided"),
                    ],
                    [
                      translateApp("Reported client"),
                      preview.client_app ?? preview.client_kind,
                    ],
                    [
                      translateApp("Platform"), preview.client_platform ?? translateApp("Not identified"),
                    ],
                    [
                      translateApp("Requested"),
                      `${formatDeviceLoginRelativeTime(preview.initiated_at, now)} - ${new Date(preview.initiated_at).toLocaleString(appLocale())}`,
                    ],
                    [
                      translateApp("Expires"),
                      waitingForDecision
                        ? t("Waiting for NyxID…")
                        : translateApp("{{date}} ({{minutes}}s)", {
                            date: new Date(preview.expires_at).toLocaleString(
                              appLocale(),
                            ),
                            minutes: remaining ?? 0,
                          }),
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
                    {translateApp(
                      "Only approve a request you started. Reported device details and a matching network do not prove who is requesting access.",
                    )}
                  </Text>
                </View>
              )}
              {expired && (
                <Text accessibilityRole="alert" style={styles.fieldErrorText}>
                  {translateApp(
                    "This request has expired. Start Agent Key login again.",
                  )}
                </Text>
              )}
              {/* A terminal code chooses its access from an iOS grouped list. */}
              {mint && step === "review" && isAuthenticated ? (
                <SettingsGroup
                  title={translateApp("Sign the terminal in with")}
                  footer={translateApp(
                    "Anyone with the code can use it once, within five minutes. An Agent Key keeps the terminal to that key's services and approvals.",
                  )}
                >
                  <SettingsRow
                    icon={KeyRound}
                    label={translateApp("Choose an Agent Key")}
                    badge={translateApp("Recommended")}
                    detail={translateApp(
                      "Only this key's services and approvals",
                    )}
                    disabled={pending || expired}
                    onPress={() => void act(loadOptions)}
                  />
                  <SettingsRow
                    icon={UserRound}
                    label={translateApp("Full account session")}
                    detail={translateApp(
                      "Signs the terminal in as you, with full access",
                    )}
                    disabled={pending}
                    onPress={() => setStep("account")}
                  />
                </SettingsGroup>
              ) : null}
              {step === "review" &&
                !(mint && isAuthenticated) &&
                (isAuthenticated &&
                (flow === "agent-key" ||
                  preview?.supports_grant_choice === true) ? (
                  <AuthSubmitButton
                    loading={pending}
                    label={translateApp("Choose an Agent Key")}
                    disabled={pending || expired}
                    onPress={() => void act(loadOptions)}
                  />
                ) : !isAuthenticated ? (
                  <AuthSubmitButton
                    label={translateApp("Sign in on this phone")}
                    onPress={() => navigation.navigate("Auth")}
                  />
                ) : (
                  <AuthSubmitButton
                    label={translateApp("Review account login")}
                    onPress={() =>
                      navigation.navigate("DeviceLogin", { user_code: code })
                    }
                  />
                ))}

              {step === "account" && (
                <View style={styles.inputSection}>
                  <Text style={styles.inputLabel}>
                    {translateApp("Confirm full account access")}
                  </Text>
                  <Text style={styles.cautionText}>
                    {translateApp(
                      "This code grants access to your account, services, credentials, and organization permissions. The session can refresh until it expires or you revoke it.",
                    )}
                  </Text>
                  <AuthSubmitButton
                    loading={pending}
                    label={translateApp("Generate account login code")}
                    disabled={pending}
                    onPress={() =>
                      void act(
                        () => createCode({ auth_kind: "account_session" }),
                        false,
                        true,
                      )
                    }
                  />
                  <PrimaryButton
                    label={translateApp("Back")}
                    kind="ghost"
                    disabled={pending}
                    onPress={() => setStep("review")}
                  />
                </View>
              )}
              {step === "options" && options && (
                <View style={styles.inputSection}>
                  <WorkspaceSelectionField
                    surfaceColors={colors}
                    label={translateApp("Agent Key")}
                    title={translateApp("Choose an Agent Key")}
                    description={translateApp(
                      "Choose a key to review its full effective permissions before approving.",
                    )}
                    placeholder={translateApp("Select an existing key")}
                    confirmLabel={translateApp("Review permissions")}
                    options={options.keys.map((key) => ({
                      value: key.id,
                      label: key.name,
                      description: (key.expires_at ? translateApp("{{name}} · {{scopes}} · {{keyPrefix}} · Expires {{time}}", { name: key.owner_name, scopes: key.scopes, keyPrefix: key.key_prefix, time: new Date(key.expires_at).toLocaleDateString(appLocale()) }) : translateApp("{{name}} · {{scopes}} · {{keyPrefix}} · No expiry", { name: key.owner_name, scopes: key.scopes, keyPrefix: key.key_prefix })),
                      searchText: `${key.id} ${key.owner_id} ${key.platform ?? ""}`,
                    }))}
                    value={[]}
                    summary={
                      selection?.kind === "existing" ? summary?.name : undefined
                    }
                    disabled={pending || expired}
                    scopeKey={`${flow}:${mint}:${code}`}
                    onChange={([id]) => {
                      if (pending || expired) return;
                      const key = options.keys.find((item) => item.id === id);
                      if (!key) return;
                      setSelection({ kind: "existing", api_key_id: key.id });
                      setSummary(key);
                      setStep("confirm");
                    }}
                  />
                  <AuthSubmitButton
                    label={translateApp("Create a new key")}
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
                    {translateApp("Confirm effective permissions")}
                  </Text>
                  {permissions(summary)}
                  <Text style={styles.cautionText}>
                    {issuanceNotice(selection?.kind === "existing")}
                  </Text>
                  <Text style={styles.inputLabel}>
                    {translateApp(
                      "Credential expiry (blank: 90 days, bounded by key expiry)",
                    )}
                  </Text>
                  <AuthTextInput
                    accessibilityLabel={translateApp("Credential expiry")}
                    placeholder={"YYYY-MM-DDTHH:mm:ssZ"}
                    placeholderTextColor={colors.textMuted}
                    value={credentialExpiry}
                    editable={!pending}
                    onChangeText={setCredentialExpiry}
                    style={[
                      styles.codeInput,
                      { textAlign: "left", fontSize: 14 },
                    ]}
                  />
                  <AuthSubmitButton
                    loading={pending}
                    label={
                      mint
                        ? translateApp("Generate restricted login code")
                        : translateApp("Approve")
                    }
                    disabled={pending || expired}
                    onPress={() => void act(() => decide(true), false, true)}
                  />
                  <PrimaryButton
                    label={translateApp("Back to keys")}
                    kind="ghost"
                    disabled={pending}
                    onPress={() => setStep("options")}
                  />
                </View>
              )}
              {preview && isAuthenticated && (
                <PrimaryButton
                  label={translateApp("Reject")}
                  kind="danger"
                  disabled={pending || expired}
                  onPress={() => void act(() => decide(false), false, true)}
                />
              )}
            </>
          )}
          {error && (
            <Text accessibilityRole="alert" style={styles.fieldErrorText}>
              {error}
            </Text>
          )}
          {decisionUncertain ? (
            <View style={{ gap: 16 }}>
              <PrimaryButton
                label={t("Done")}
                kind="ghost"
                disabled={writing}
                onPress={back}
              />
              <PrimaryButton
                label={t("Scan or enter another code")}
                kind="ghost"
                disabled={writing}
                onPress={restart}
              />
            </View>
          ) : null}
        </ScrollView>
      </KeyboardAvoidingView>
    </ScreenContainer>
  );
}
