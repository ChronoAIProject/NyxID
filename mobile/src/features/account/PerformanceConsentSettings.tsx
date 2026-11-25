import { useEffect, useRef, useState } from "react";
import { Switch } from "react-native";
import { ChartColumn } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import {
  getPerformanceConsentEpoch,
  hydratePerformanceConsent,
  isPerformanceConfigured,
  setPerformanceConsent,
  subscribePerformanceConsent,
} from "../../lib/performance/metrics";

/** This installation's optional timing consent is independent of account analytics. */
export function PerformanceConsentSettings() {
  const { t } = useAppLanguage();
  const configured = isPerformanceConfigured();
  const [enabled, setEnabled] = useState(
    () => getPerformanceConsentEpoch() !== null,
  );
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<"load" | "enable" | "disable" | null>(
    null,
  );
  const [loadAttempt, setLoadAttempt] = useState(0);
  const mounted = useRef(false);
  const pending = useRef(false);

  useEffect(() => {
    if (!configured) return;
    let live = true;
    mounted.current = true;
    const update = () => {
      if (!live) return;
      const next = getPerformanceConsentEpoch() !== null;
      setEnabled(next);
      if (next) setError(null);
    };
    const unsubscribe = subscribePerformanceConsent(update);
    update();
    void hydratePerformanceConsent()
      .then(() => {
        if (!live) return;
        update();
        setLoaded(true);
        setError(null);
      })
      .catch(() => {
        if (!live) return;
        update();
        setError("load");
      });
    return () => {
      live = false;
      mounted.current = false;
      unsubscribe();
    };
  }, [configured, loadAttempt]);

  const save = async (next: boolean) => {
    if (!loaded || pending.current) return;
    pending.current = true;
    setSaving(true);
    setError(null);
    try {
      await setPerformanceConsent(next);
    } catch {
      if (mounted.current) setError(next ? "enable" : "disable");
    } finally {
      pending.current = false;
      if (mounted.current) {
        setEnabled(getPerformanceConsentEpoch() !== null);
        setSaving(false);
      }
    }
  };

  if (!configured) return null;
  return (
    <SettingsGroup
      title={t("Performance metrics")}
      footer={t(
        "Share app launch, first frame, input readiness, chat response times, frame rate, JavaScript responsiveness, and peak app memory use from this device. This setting is independent of account usage analytics and is off by default.",
      )}
    >
      <SettingsRow
        icon={ChartColumn}
        label={t("Share performance metrics")}
        accessory={
          <Switch
            accessibilityLabel={t("Share performance metrics on this device")}
            accessibilityState={{
              busy: saving || (!loaded && error !== "load"),
            }}
            value={enabled}
            disabled={!loaded || saving}
            onValueChange={(value) => void save(value)}
          />
        }
        chevron={false}
      />
      {error && (
        <SettingsRow
          label={t(
            error === "load"
              ? "Retry loading performance choice"
              : "Retry saving performance choice",
          )}
          detail={t(
            error === "disable"
              ? "Performance sharing is off for this session. This choice could not be saved; retry before closing the app."
              : error === "enable"
                ? "Performance sharing remains off. Your choice could not be saved; try again."
                : "Your performance choice could not be loaded. Sharing remains off; try again.",
          )}
          onPress={() => {
            if (error === "load") {
              setError(null);
              setLoadAttempt((attempt) => attempt + 1);
            } else void save(error === "enable");
          }}
          disabled={saving}
          chevron={false}
        />
      )}
    </SettingsGroup>
  );
}
