import { useEffect, useState } from "react";
import { Switch } from "react-native";
import { useMutation, useQuery } from "@tanstack/react-query";
import { ChartColumn, Smartphone } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useMobileConsent } from "../../lib/consent";
import { analyticsConsentState } from "../../lib/analyticsConsentState";
import {
  saveAnalyticsPermission,
  type AnalyticsPermission,
} from "../../lib/analyticsRelay";
import { requestJson } from "../../lib/api/http";

/** Settings › Privacy: optional usage analytics for the account and this device. */
export function PrivacySettingsScreen(
  _props: SheetScreenProps<"PrivacySettings">,
) {
  const [toast, setToast] = useState<ToastState | null>(null);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);
  const deviceConsent = useMobileConsent();
  const analyticsPermission = useQuery({
    queryKey: ["analytics-consent", deviceConsent.accountId],
    enabled: Boolean(deviceConsent.accountId),
    staleTime: 0,
    queryFn: async () => {
      const accountId = deviceConsent.accountId!;
      const permission = await requestJson<AnalyticsPermission>(
        "/users/me/preferences/analytics",
        { expectedAccountId: accountId },
      );
      if (permission.account_id !== accountId)
        throw new Error("analytics_account_changed");
      return permission;
    },
  });
  const saveAnalytics = useMutation({
    mutationFn: (enabled: boolean) =>
      saveAnalyticsPermission(
        deviceConsent.accountId!,
        enabled,
        requestJson,
        analyticsConsentState.getSnapshot,
        deviceConsent.setConsent,
      ),
    onSuccess: () => {
      void analyticsPermission.refetch();
    },
    onError: () =>
      setToast({
        message:
          "Could not save account analytics permission. This device remains off; retry to withdraw across devices.",
        kind: "error",
      }),
  });
  const accountLabel = analyticsPermission.isFetching
    ? "Loading permission…"
    : analyticsPermission.isError
      ? "Permission unavailable"
      : analyticsPermission.data?.enabled
        ? "Account permission enabled"
        : "Account permission disabled";
  return (
    <>
      <SettingsPage>
        <SettingsGroup
          title="Usage analytics"
          footer="Optional events require account permission and this device's choice. Security and billing records have separate retention rules."
        >
          <SettingsRow
            icon={ChartColumn}
            label={accountLabel}
            accessory={
              <Switch
                accessibilityLabel="Allow account usage analytics"
                value={analyticsPermission.data?.enabled === true}
                disabled={
                  saveAnalytics.isPending ||
                  analyticsPermission.isFetching ||
                  analyticsPermission.isError ||
                  !analyticsPermission.data
                }
                onValueChange={(value) => saveAnalytics.mutate(value)}
              />
            }
          />
          <SettingsRow
            icon={Smartphone}
            label="Allow on this device"
            accessory={
              <Switch
                accessibilityLabel="Allow usage analytics on this device"
                value={deviceConsent.enabled}
                disabled={
                  !deviceConsent.enabled &&
                  (analyticsPermission.data?.enabled !== true ||
                    analyticsPermission.isError ||
                    analyticsPermission.isFetching)
                }
                onValueChange={deviceConsent.setConsent}
              />
            }
          />
        </SettingsGroup>
        {analyticsPermission.isError && (
          <PrimaryButton
            refreshLabelOnFontScaleChange
            label="Retry"
            kind="ghost"
            onPress={() => {
              void analyticsPermission.refetch();
            }}
          />
        )}
        {deviceConsent.persistenceError && (
          <SettingsGroup footer="This device's choice could not be saved. Analytics is off in this session; retry to save it before closing the app.">
            <SettingsRow
              label="Retry saving device choice"
              onPress={deviceConsent.retryPersistence}
              chevron={false}
            />
          </SettingsGroup>
        )}
      </SettingsPage>
      <ToastOverlay toast={toast} />
    </>
  );
}
