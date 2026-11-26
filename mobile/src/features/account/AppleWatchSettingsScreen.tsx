import { useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "react-native";
import { Watch } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { getApiBaseUrl } from "../../lib/api/baseUrl";
import { getFrontendUrl } from "../../lib/env";
import { loginCodeApi } from "../../lib/api/loginCodeApi";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import {
  nativeWatchLink,
  type WatchLinkStatus,
} from "../../lib/watch/watchLink";
import { useMfaStepUp } from "../auth/useMfaStepUp";
import {
  displayWatchCode,
  signOutMessage,
  watchAvailability,
  watchSetupCode,
  WatchPairingFlow,
  type WatchPairingState,
} from "./watchPairing";

/**
 * Settings › Apple Watch: signs the NyxID watch app in with its own,
 * separately revocable session, or signs it out.
 */
export function AppleWatchSettingsScreen(
  _props: SheetScreenProps<"AppleWatchSettings">,
) {
  const { t } = useAppLanguage();
  const { isConnected } = useNetworkStatus();
  const link = useMemo(() => nativeWatchLink(), []);
  const [status, setStatus] = useState<WatchLinkStatus | null>(null);
  const [pairing, setPairing] = useState<WatchPairingState>({ step: "idle" });
  const [notice, setNotice] = useState<string | null>(null);
  const stepUp = useMfaStepUp(`apple-watch:${getAuthAccountId() ?? ""}`);
  const stepUpRef = useRef(stepUp);
  stepUpRef.current = stepUp;
  const flow = useRef<WatchPairingFlow | null>(null);

  useEffect(() => {
    if (!link) return;
    let live = true;
    void link
      .getStatus()
      .then((next) => {
        if (live) setStatus(next);
      })
      .catch(() => {});
    const subscription = link.addListener("onStatusChange", (next) => {
      if (live) setStatus(next);
    });
    const pairingFlow = new WatchPairingFlow({
      api: {
        mint: () => loginCodeApi.mint({ auth_kind: "account_session" }),
        status: (id) => loginCodeApi.status(id),
        cancel: (id) => loginCodeApi.cancel(id),
      },
      link,
      apiBaseUrl: getApiBaseUrl,
      frontendUrl: getFrontendUrl,
      notify: (state) => {
        if (live) setPairing(state);
      },
      handleMintError: (error, retry) => stepUpRef.current.handle(error, retry),
    });
    flow.current = pairingFlow;
    return () => {
      live = false;
      subscription.remove();
      pairingFlow.stop();
      flow.current = null;
    };
  }, [link]);

  const availability = link ? watchAvailability(status) : "unsupported";
  const watchCode = watchSetupCode(status);
  const detail = statusDetail(availability, watchCode !== null);
  const busy =
    pairing.step === "minting" ||
    pairing.step === "sending" ||
    pairing.step === "waiting";

  const connect = () => {
    if (!watchCode) return;
    setNotice(null);
    void flow.current?.start(watchCode);
  };

  const disconnect = () => {
    if (!link) return;
    Alert.alert(
      t("Sign out of your Apple Watch?"),
      t("NyxID on your watch signs out. Your iPhone stays signed in."),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: t("Sign out watch"),
          style: "destructive",
          onPress: () => {
            void link
              .send(signOutMessage())
              .then((delivery) => {
                setPairing({ step: "idle" });
                setNotice(
                  delivery === "delivered"
                    ? t("Your watch is signed out.")
                    : t("Your watch signs out the next time NyxID opens on it."),
                );
              })
              .catch(() =>
                setNotice(
                  t(
                    "Couldn't reach your watch. To end its session now, sign it out from Account & security › Sessions.",
                  ),
                ),
              );
          },
        },
      ],
    );
  };

  if (stepUp.prompt) return <SettingsPage>{stepUp.prompt}</SettingsPage>;

  return (
    <SettingsPage>
      {!isConnected && (
        <Notice
          message={t("You're offline. Reconnect to set up your watch.")}
        />
      )}
      <SettingsGroup
        title={t("Apple Watch")}
        footer={t(
          "Your watch gets its own sign-in for approvals, notifications and talking to your agents. Sign it out here, on the watch, or from Account & security › Sessions.",
        )}
      >
        <SettingsRow
          icon={Watch}
          label={t(statusLabel(availability))}
          detail={detail ? t(detail) : undefined}
        />
        {availability === "ready" && watchCode ? (
          <SettingsRow
            label={t("Code on your watch")}
            value={displayWatchCode(watchCode)}
          />
        ) : null}
        {availability === "ready" ? (
          <SettingsRow
            tone="action"
            label={
              pairing.step === "connected"
                ? t("Connect again")
                : t("Connect Apple Watch")
            }
            disabled={busy || !isConnected || !watchCode}
            onPress={connect}
          />
        ) : null}
      </SettingsGroup>

      {pairingNotice(pairing) ? (
        <Notice
          tone={
            pairing.step === "failed"
              ? "error"
              : pairing.step === "connected"
                ? "success"
                : "info"
          }
          message={t(pairingNotice(pairing)!)}
        />
      ) : null}
      {notice ? <Notice message={notice} /> : null}

      {availability === "ready" || availability === "not_installed" ? (
        <SettingsGroup>
          <SettingsRow
            tone="danger"
            label={t("Sign out of Apple Watch")}
            disabled={busy}
            onPress={disconnect}
          />
        </SettingsGroup>
      ) : null}
    </SettingsPage>
  );
}

function statusLabel(availability: ReturnType<typeof watchAvailability>) {
  switch (availability) {
    case "checking":
      return "Checking for your watch…";
    case "unsupported":
      return "Apple Watch isn't available on this device";
    case "no_watch":
      return "No Apple Watch paired";
    case "not_installed":
      return "NyxID isn't on your watch yet";
    case "ready":
      return "Apple Watch paired";
  }
}

function statusDetail(
  availability: ReturnType<typeof watchAvailability>,
  hasCode: boolean,
): string | null {
  switch (availability) {
    case "no_watch":
      return "Pair a watch in the Watch app on this iPhone.";
    case "not_installed":
      return "Install it from the Watch app › Available Apps, then come back.";
    case "ready":
      return hasCode
        ? "Check the code matches your watch, then tap Connect."
        : "Open NyxID on your watch to see its code.";
    default:
      return null;
  }
}

export function pairingNotice(state: WatchPairingState): string | null {
  switch (state.step) {
    case "minting":
      return "Creating a one-time sign-in code…";
    case "sending":
      return "Sending it to your watch…";
    case "waiting":
      return state.delivery === "delivered"
        ? "Waiting for your watch to sign in…"
        : "Open NyxID on your watch to finish signing in.";
    case "connected":
      return "Your watch is signed in.";
    case "failed":
      return state.message;
    case "idle":
      return null;
  }
}
