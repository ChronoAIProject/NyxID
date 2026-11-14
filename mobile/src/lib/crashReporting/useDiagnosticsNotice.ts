import { useEffect } from "react";
import { Alert } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useMobileConsent } from "../consent";
import { requestJson } from "../api/http";
import type { AnalyticsPermission } from "../analyticsRelay";
import { useAppLanguage } from "../../i18n/LanguageProvider";
export function useDiagnosticsNotice(
  authenticated: boolean,
  onReview: () => void,
) {
  const { accountId, isHydrated, enabled, policy } = useMobileConsent();
  const { t } = useAppLanguage();
  useEffect(() => {
    if (!authenticated || !accountId || !isHydrated || !enabled) return;
    let active = true;
    const key = `nyxid.diagnostics.notice.${encodeURIComponent(accountId)}`;
    void (async () => {
      if (await AsyncStorage.getItem(key)) return;
      const p = await requestJson<AnalyticsPermission>(
        "/users/me/preferences/analytics",
        { expectedAccountId: accountId },
      );
      if (
        !active ||
        p.account_id !== accountId ||
        !p.enabled ||
        p.diagnostics_offered !== true ||
        p.current_policy_version !== "2" ||
        (p.policy_version === "2" && policy === 2)
      )
        return;
      await AsyncStorage.setItem(key, "2");
      if (!active) return;
      Alert.alert(
        t("Analytics updated"),
        t(
          "Optional crash diagnostics include app version and sanitized code locations, without messages or URLs. Reports are kept for up to 30 days. Review to allow updated analytics, or keep your current choice.",
        ),
        [
          { text: t("Not now"), style: "cancel" },
          {
            text: t("Review"),
            onPress: () => {
              if (active) onReview();
            },
          },
        ],
      );
    })().catch(() => {});
    return () => {
      active = false;
    };
  }, [authenticated, accountId, isHydrated, enabled, policy, t, onReview]);
}
