import { localizedToast } from "../../i18n/localizedToast";
import { translateApp } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useEffect, useState } from "react";
import { Alert, Switch } from "react-native";
import { useMutation, useQuery } from "@tanstack/react-query";
import { ChartColumn, Download, Smartphone } from "lucide-react-native";
import { Directory, File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
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
import { requestJson, requestBoundAuthenticatedResponse } from "../../lib/api/http";
import { ApiError } from "../../lib/api/ApiError";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import { cleanStaleAccountExports, shareAccountExport } from "./accountExport";
import { PerformanceConsentSettings } from "./PerformanceConsentSettings";

/** Settings › Privacy: optional usage analytics for the account and this device. */
export function PrivacySettingsScreen(
  _props: SheetScreenProps<"PrivacySettings">,
) {
  const { t } = useAppLanguage();
  const [toast, setToast] = useState<ToastState | null>(null);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);
  useEffect(() => {
    try { cleanStaleAccountExports(new Directory(Paths.cache).list()); } catch { /* Retry on next mount. */ }
  }, []);
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
  const diagnosticsEnabled =
    analyticsPermission.data?.diagnostics_available &&
    deviceConsent.enabled &&
    deviceConsent.policy === 2;
  const saveAnalytics = useMutation({
    mutationFn: (enabled: boolean) =>
      saveAnalyticsPermission(
        deviceConsent.accountId!,
        enabled,
        requestJson,
        analyticsConsentState.getSnapshot,
        deviceConsent.setConsent,
        analyticsPermission.data?.diagnostics_offered &&
          (analyticsPermission.data?.policy_version === "2" ||
            deviceConsent.policy === 2)
          ? 2
          : 1,
      ),
    onSuccess: () => {
      void analyticsPermission.refetch();
    },
    onError: () =>
      setToast(localizedToast(() => ({
        message: translateApp(
          "Could not save account analytics permission. This device remains off; retry to withdraw across devices.",
        ),
        kind: "error",
      }))),
  });
  const exportData = useMutation({
    mutationFn: async () => {
      const accountId = getAuthAccountId();
      if (!accountId) throw new Error("auth_session_missing");
      await shareAccountExport({
        available: Sharing.isAvailableAsync,
        request: (signal) => requestBoundAuthenticatedResponse("/users/me/export", {
          method: "POST", expectedAccountId: accountId, signal,
        }),
        create: () => {
          const file = new File(Paths.cache, `nyxid-account-export-${Date.now()}.json`);
          file.create();
          let handle: ReturnType<File["open"]>;
          try { handle = file.open(); } catch (error) { file.delete(); throw error; }
          return {
            uri: file.uri,
            write: (bytes) => handle.writeBytes(bytes),
            close: () => handle.close(),
            remove: () => { if (file.exists) file.delete(); },
          };
        },
        share: (uri) => Sharing.shareAsync(uri, {
          mimeType: "application/json", UTI: "public.json", dialogTitle: t("Export my data"),
        }),
      });
    },
    onSuccess: () =>
      setToast(localizedToast(() => ({ message: translateApp("Export share sheet closed."), kind: "info", lines: 2 }))),
    onError: (error: unknown) => {
      if (error instanceof ApiError && error.statusCode === 429) {
        const seconds = error.retryAfterMs ? Math.ceil(error.retryAfterMs / 1000) : 0;
        const minutes = seconds
          ? Math.max(1, Math.ceil(seconds / 60))
          : undefined;
        setToast(localizedToast(() => ({
          message:
            minutes === undefined
              ? translateApp("Your data export is temporarily rate limited.")
              : seconds && seconds < 60
                ? seconds === 1
                  ? translateApp("Your data export is temporarily rate limited. Try again in 1 second.")
                  : translateApp("Your data export is temporarily rate limited. Try again in {{seconds}} seconds.", { seconds })
                : minutes === 1
                ? translateApp(
                    "Your data export is temporarily rate limited. Try again in 1 minute.",
                  )
                : translateApp(
                    "Your data export is temporarily rate limited. Try again in {{minutes}} minutes.",
                    { minutes },
                  ),
          kind: "error",
          lines: 2,
        })));
      } else if (
        error instanceof ApiError &&
        (error.statusCode === 404 || error.statusCode === 405)
      ) {
        setToast(localizedToast(() => ({
          message: translateApp("Data export is not available on this server yet."),
          kind: "error",
          lines: 2,
        })));
      } else {
        setToast(localizedToast(() => ({
          message: translateApp("We couldn't create your data export. Please try again."),
          kind: "error",
          lines: 2,
        })));
      }
    },
  });
  const reviewDiagnostics = useMutation({
    mutationFn: () =>
      saveAnalyticsPermission(
        deviceConsent.accountId!,
        true,
        requestJson,
        analyticsConsentState.getSnapshot,
        deviceConsent.setConsent,
        2,
      ),
    onSuccess: () => {
      void analyticsPermission.refetch();
    },
    onError: () =>
      setToast({
        message: t("Could not save updated analytics permission."),
        kind: "error",
      }),
  });
  const review = () =>
    Alert.alert(
      t("Review updated analytics"),
      t(
        "Optional crash diagnostics include app version and sanitized code locations, without messages or URLs. Reports are kept for up to 30 days. Review to allow updated analytics, or keep your current choice.",
      ),
      [
        { text: t("Not now"), style: "cancel" },
        {
          text: t("Allow updated analytics"),
          onPress: () => reviewDiagnostics.mutate(),
        },
      ],
    );
  // The row names the setting; the switch shows its state. Only an error
  // needs words; a load disables the switch without placeholder text.
  const accountDetail = analyticsPermission.isFetching
    ? undefined
    : analyticsPermission.isError
      ? translateApp("Permission unavailable")
      : undefined;
  return (
    <>
      <SettingsPage>
        <SettingsGroup
          title={t("Your data")}
          footer={t(
            "Download a JSON copy of your account profile, services, activity, your assistant inputs, private assistant replies to web/mobile turns without platform routing, and channel metadata, session metadata, machine metadata and billing summary. Passwords, tokens, keys, secrets, raw files and transient coordination records are excluded; large sections may be truncated.",
          )}
        >
          <SettingsRow
            icon={Download}
            label={t("Export my data")}
            detail={t(
              "Creates a bounded JSON export. You can request one up to three times per hour.",
            )}
            onPress={() => exportData.mutate()}
            disabled={exportData.isPending}
            accessibilityState={{
              disabled: exportData.isPending,
              busy: exportData.isPending,
            }}
          />
        </SettingsGroup>
        <SettingsGroup
          title={t("Usage analytics")}
          footer={t(
            "Optional usage events are sent only when both your account and this device allow them. Security and billing records have separate retention rules.",
          )}
        >
          <SettingsRow
            icon={ChartColumn}
            label={t("Allow for my account")}
            detail={accountDetail ? t(accountDetail) : undefined}
            accessory={
              <Switch
                accessibilityLabel={t("Allow account usage analytics")}
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
            label={t("Allow on this device")}
            accessory={
              <Switch
                accessibilityLabel={t("Allow usage analytics on this device")}
                value={deviceConsent.enabled}
                disabled={
                  !deviceConsent.enabled &&
                  (analyticsPermission.data?.enabled !== true ||
                    analyticsPermission.isError ||
                    analyticsPermission.isFetching)
                }
                onValueChange={(value) =>
                  deviceConsent.setConsent(
                    value,
                    deviceConsent.policy === 2 ? 2 : 1,
                  )
                }
              />
            }
          />
        </SettingsGroup>
        <PerformanceConsentSettings />
        {analyticsPermission.data?.diagnostics_offered === true && (
          <SettingsGroup
            title={t("Crash diagnostics")}
            footer={t(
              "iOS includes native crashes and JavaScript errors. Android reports JavaScript errors and JavaScript fatal errors only; JVM and NDK crashes are not captured.",
            )}
          >
            <SettingsRow
              icon={ChartColumn}
              label={t("Crash diagnostics")}
              detail={t(
                diagnosticsEnabled
                  ? "Crash diagnostics enabled"
                  : "Crash diagnostics need updated permission",
              )}
              chevron={false}
            />
            {!diagnosticsEnabled && (
              <SettingsRow
                label={t("Review updated analytics")}
                onPress={
                  analyticsPermission.data?.current_policy_version === "2" &&
                  !reviewDiagnostics.isPending &&
                  !saveAnalytics.isPending
                    ? review
                    : undefined
                }
              />
            )}
          </SettingsGroup>
        )}
        {analyticsPermission.isError && (
          <PrimaryButton
            refreshLabelOnFontScaleChange
            label={t("Retry")}
            kind="ghost"
            onPress={() => {
              void analyticsPermission.refetch();
            }}
          />
        )}
        {deviceConsent.persistenceError && (
          <SettingsGroup
            footer={t(
              "This device's choice could not be saved. Analytics is off in this session; retry to save it before closing the app.",
            )}
          >
            <SettingsRow
              label={t("Retry saving device choice")}
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
