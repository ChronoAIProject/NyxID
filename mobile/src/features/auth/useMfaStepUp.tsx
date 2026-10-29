import { useEffect, useRef, useState } from "react";
import { AppState, View, Text, TextInput } from "react-native";
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
  const [busy, setBusy] = useState(false);
  const active = useRef<string | null>(scope);
  const lifecycle = useRef(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    active.current = scope;
    const subscription = AppState.addEventListener("change", (state) => {
      active.current = state === "active" ? scope : null;
      if (state !== "active") {
        lifecycle.current++;
        setPending(null);
      }
    });
    return () => {
      active.current = null;
      subscription.remove();
    };
  }, [scope]);
  if (pending && !stepUpScopeIsCurrent(pending, scope, authSessionGeneration()))
    setPending(null);
  async function handle(
    error: unknown,
    retry: () => Promise<void>,
  ): Promise<boolean> {
    const expectedLifecycle = lifecycle.current;
    const isCurrent = () =>
      expectedLifecycle === lifecycle.current &&
      stepUpScopeIsCurrent(
        { scope, generation },
        active.current,
        authSessionGeneration(),
      );
    const decision = await decideStepUp(
      error,
      () => mobileApi.getAccountProfile(),
      isCurrent,
    );
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
  const promptLifecycle = lifecycle.current;
  const prompt =
    pending && stepUpScopeIsCurrent(pending, scope, authSessionGeneration()) ? (
      <MfaStepUpPrompt
        message={pending.message}
        cancel={() => setPending(null)}
        verify={async (code) => {
          setBusy(true);
          try {
            await completeStepUp(
              code,
              async (value) => {
                await requestJson("/auth/mfa/step-up", {
                  method: "POST",
                  body: { code: value },
                  retryOnAuthFailure: false,
                  invalidateSessionOnAuthFailure: false,
                });
              },
              async () => {
                setPending(null);
                await pending.retry();
              },
              () =>
                promptLifecycle === lifecycle.current &&
                stepUpScopeIsCurrent(
                  pending,
                  active.current,
                  authSessionGeneration(),
                ),
            );
          } finally {
            if (mounted.current) setBusy(false);
          }
        }}
      />
    ) : null;
  return { handle, prompt, busy };
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
  const mounted = useRef(true);
  const submitting = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function submit() {
    if (submitting.current || !code.trim()) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      await verify(code.trim());
    } catch (failure) {
      if (mounted.current)
        setError(
          failure instanceof Error
            ? failure.message
            : "Verification failed. Try again.",
        );
    } finally {
      submitting.current = false;
      if (mounted.current) {
        setBusy(false);
        setCode("");
      }
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
