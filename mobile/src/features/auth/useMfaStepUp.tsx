import { useEffect, useRef, useState } from "react";
import { View, Text, TextInput } from "react-native";
import { requestJson } from "../../lib/api/http";
import { mobileApi } from "../../lib/api/mobileApi";
import { authSessionGeneration } from "../../lib/auth/sessionStore";
import { PrimaryButton } from "../../components/PrimaryButton";
import { spacing } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import {
  decideStepUp,
  stepUpScopeIsCurrent,
  completeStepUp,
} from "./mfaStepUp";

export function useMfaStepUp(scope: string) {
  // Keep the originating login even when its refusal arrives after a switch.
  const generation = authSessionGeneration();
  const [pending, setPending] = useState<{
    retry: () => Promise<void>;
    message: string;
    generation: number;
    scope: string;
  } | null>(null);
  const active = useRef<string | null>(scope);
  useEffect(() => {
    active.current = scope;
    return () => {
      active.current = null;
    };
  }, [scope]);
  if (
    pending &&
    !stepUpScopeIsCurrent(pending, scope, authSessionGeneration())
  )
    setPending(null);
  async function handle(
    error: unknown,
    retry: () => Promise<void>,
  ): Promise<boolean> {
    const isCurrent = () =>
      stepUpScopeIsCurrent({ scope, generation }, active.current, authSessionGeneration());
    const decision = await decideStepUp(error, () => mobileApi.getAccountProfile(), isCurrent);
    if (decision === "discard") return true;
    if (decision !== "prompt") return false;
    setPending({
      retry,
      message: error instanceof Error ? error.message : "Verification required",
      generation,
      scope,
    });
    return true;
  }
  const prompt =
    pending && stepUpScopeIsCurrent(pending, scope, authSessionGeneration()) ? (
      <MfaStepUpPrompt
        message={pending.message}
        cancel={() => setPending(null)}
        verify={async (code) => {
          await completeStepUp(
            code,
            async (value) => {
              await requestJson("/auth/mfa/step-up", {
                method: "POST",
                body: { code: value },
                retryOnAuthFailure: false,
              });
            },
            async () => {
              setPending(null);
              await pending.retry();
            },
            () =>
              stepUpScopeIsCurrent(pending, active.current, authSessionGeneration()),
          );
        }}
      />
    ) : null;
  return { handle, prompt };
}

function MfaStepUpPrompt({
  message,
  cancel,
  verify,
}: {
  message: string;
  cancel: () => void;
  verify: (code: string) => Promise<void>;
}) {
  const { colors } = useTheme();
  const styles = createDeviceLoginStyles(colors);
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function submit() {
    if (busy || !code.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await verify(code.trim());
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Verification failed. Try again.",
      );
    } finally {
      setBusy(false);
      setCode("");
    }
  }
  return (
    <View
      style={[styles.detailPanel, { padding: spacing.xxl, gap: spacing.lg }]}
      accessibilityLabel="Verify this session"
    >
      <Text style={[styles.title, { flex: 0 }]}>Verify this session</Text>
      <Text style={styles.signInText}>{message}</Text>
      <TextInput
        accessibilityLabel={recovery ? "Recovery code" : "Authenticator code"}
        value={code}
        onChangeText={setCode}
        style={styles.codeInput}
        maxLength={recovery ? 32 : 6}
        keyboardType={recovery ? "default" : "number-pad"}
        autoCapitalize="none"
        autoCorrect={false}
        editable={!busy}
        secureTextEntry={recovery}
      />
      {error && (
        <Text accessibilityRole="alert" style={styles.errorText}>
          {error}
        </Text>
      )}
      <PrimaryButton
        kind="ghost"
        disabled={busy}
        label={recovery ? "Use an authenticator code" : "Use a recovery code"}
        onPress={() => {
          setRecovery(!recovery);
          setCode("");
        }}
      />
      <PrimaryButton
        disabled={busy || !code.trim()}
        label={busy ? "Verifying..." : "Verify & continue"}
        onPress={() => void submit()}
      />
      <PrimaryButton
        kind="ghost"
        disabled={busy}
        label="Cancel"
        onPress={cancel}
      />
    </View>
  );
}
