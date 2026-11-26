import { useMemo } from "react";
import { Text, View } from "react-native";
import { ScreenContainer } from "../../components/ScreenContainer";
import { PrimaryButton } from "../../components/PrimaryButton";
import { MagicKeyIllustration } from "../../components/icons/empty-state/MagicKeyIllustration";
import { RoadBarrierIllustration } from "../../components/icons/empty-state/RoadBarrierIllustration";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { AuthSubmitButton } from "./AuthSubmitButton";
import { useAuthTheme } from "./useAuthTheme";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import type { LoginTerminal } from "./loginRequestResolver";

export function LoginRequestTerminalView({
  state,
  clientKind,
  inSheet,
  onDone,
  onRestart,
  disabled = false,
}: {
  state: LoginTerminal;
  clientKind?: string;
  inSheet: boolean;
  onDone: () => void;
  onRestart: () => void;
  disabled?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useAuthTheme();
  const styles = useMemo(() => createDeviceLoginStyles(colors), [colors]);
  const [title, explanation] = (
    {
      approved: [
        "Login approved",
        clientKind === "browser"
          ? "Return to your browser to continue."
          : "Return to where you started signing in.",
      ],
      denied: [
        "Request denied",
        "This login request was rejected. Start a new request on the requesting device.",
      ],
      expired: [
        "Login request expired",
        "This login request has expired. Start a new request on the requesting device.",
      ],
      completed: [
        "Login request already used",
        "This login request was already used. Start a new request on the requesting device.",
      ],
      unavailable: [
        "Login request unavailable",
        "This login request is no longer available. Scan or enter another code.",
      ],
    } as const
  )[state];
  return (
    <ScreenContainer>
      <View style={[styles.terminalScreen, !inSheet && styles.rootContent]}>
        {state === "approved" ? (
          <MagicKeyIllustration
            size={224}
            color={colors.success}
            opacity={0.85}
          />
        ) : (
          <RoadBarrierIllustration
            size={224}
            color={colors.danger}
            opacity={0.85}
          />
        )}
        <Text style={styles.terminalTitle}>{t(title)}</Text>
        <Text style={styles.terminalBody}>{t(explanation)}</Text>
        <AuthSubmitButton
          label={t("Done")}
          disabled={disabled}
          onPress={onDone}
        />
        <PrimaryButton
          kind="ghost"
          disabled={disabled}
          label={t("Scan or enter another code")}
          onPress={onRestart}
        />
      </View>
    </ScreenContainer>
  );
}
