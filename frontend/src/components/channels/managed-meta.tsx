import { useAuthStore } from "@/stores/auth-store";
import { MetaCustomerSetup } from "./meta-customer-setup";
import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useMetaConnection } from "@/hooks/use-channel-meta";
import { useChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";
import { ChannelBotHealthCard } from "./channel-bot-health-card";
import type {
  ManagedFlowProps,
  ManagedDetailProps,
} from "./managed-flow-types";
import type { CreateChannelBotResponse } from "@/types/channels";

type MetaConnectProps = Omit<ManagedFlowProps, "bootstrap"> & {
  bootstrap?: ManagedFlowProps["bootstrap"];
};

export function ManagedMetaConnect(props: MetaConnectProps) {
  const link = useChannelConnectLinkContext();
  const actor = useAuthStore((s) => s.user?.id);
  const { platform, orgId, botId } = props;
  const storageKey = `meta-inbox:${actor}:${platform}:${orgId ?? "personal"}:${botId ?? "new"}:${link?.id ?? "direct"}`;
  return <MetaConnectForm key={storageKey} {...props} link={link} storageKey={storageKey} />;
}

function MetaConnectForm({
  platform,
  bootstrap,
  label,
  orgId,
  botId,
  onConnected,
  link,
  storageKey,
}: MetaConnectProps & {
  link: ReturnType<typeof useChannelConnectLinkContext>;
  storageKey: string;
}) {
  const { t } = useUiTranslation();
  const [sessionId, setSessionId] = useState<string | null>(() => {
    if (link) return link.connectionId ?? null;
    try {
      return sessionStorage.getItem(storageKey);
    } catch {
      return null;
    }
  });
  const [selected, setSelected] = useState("");
  const [exclusive, setExclusive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [popupClosed, setPopupClosed] = useState(false);
  const [observedLinkSession, setObservedLinkSession] = useState(link?.connectionId);
  const popup = useRef<Window | null>(null);
  const popupSession = useRef<string | null>(null);
  const mounted = useRef(false);
  const activeSession = useRef(sessionId);
  const connection = useMetaConnection(platform, sessionId);
  if (observedLinkSession !== link?.connectionId) {
    setObservedLinkSession(link?.connectionId);
    if (sessionId !== (link?.connectionId ?? null)) {
      setSessionId(link?.connectionId ?? null);
      setSelected("");
      setExclusive(false);
      setPopupClosed(false);
      setError(null);
    }
  }
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      popup.current?.close();
    };
  }, []);
  useEffect(() => {
    activeSession.current = sessionId;
  }, [sessionId]);
  useEffect(() => {
    try {
      if (sessionId) sessionStorage.setItem(storageKey, sessionId);
      else sessionStorage.removeItem(storageKey);
    } catch {
      /* The actor-bound server session remains authoritative. */
    }
  }, [sessionId, storageKey]);
  useEffect(() => {
    if (!sessionId || connection.progress.data?.status !== "pending") return;
    const timer = window.setInterval(() => {
      if (popupSession.current === sessionId && popup.current?.closed) setPopupClosed(true);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [sessionId, connection.progress.data?.status]);
  useEffect(() => {
    if (popupSession.current === sessionId && connection.progress.data?.status === "assets")
      popup.current?.close();
  }, [connection.progress.data?.status, sessionId]);

  async function start() {
    setError(null);
    setSelected("");
    setExclusive(false);
    popupSession.current = null;
    popup.current = window.open(
      "about:blank",
      "nyxid-meta-consent",
      "popup,width=620,height=760",
    );
    if (!popup.current) {
      setError(t("Allow popups to sign in to your inbox account."));
      return;
    }
    try {
      // Setup polls the actor-bound session and does not need a provider opener.
      popup.current.opener = null;
      const result = await connection.start.mutateAsync({
        label,
        target_org_id: orgId ?? undefined,
        reconnect_bot_id: botId,
        setup_token: link?.token,
      });
      if (!mounted.current) return;
      if (link && activeSession.current !== sessionId && activeSession.current !== result.session_id) {
        popup.current.close();
        void link.refresh();
        return;
      }
      popupSession.current = result.session_id;
      setSessionId(result.session_id);
      setPopupClosed(false);
      popup.current.location.href = result.authorization_url;
      void link?.refresh();
    } catch (failure) {
      if (!mounted.current) return;
      popup.current.close();
      setError(
        failure instanceof Error
          ? failure.message
          : t("Account sign-in could not start."),
      );
    }
  }
  async function finish(assetId: string, confirmed: boolean) {
    setError(null);
    try {
      const result = await connection.finish.mutateAsync({
        asset_id: assetId,
        exclusive_routing_verified: confirmed,
      });
      if (!mounted.current || activeSession.current !== sessionId) return;
      await link?.refresh();
      if (!mounted.current || activeSession.current !== sessionId) return;
      if (result.status === "suspended") {
        await connection.progress.refetch();
        return;
      }
      onConnected({
        ...result,
        connection_id: null,
        managed_setup: null,
        setup_instructions: [],
        webhook_secret_label: null,
      } as CreateChannelBotResponse);
    } catch (failure) {
      if (!mounted.current || activeSession.current !== sessionId) return;
      setError(
        failure instanceof Error
          ? failure.message
          : t("Inbox setup needs attention. Select Verify to retry."),
      );
      void connection.progress.refetch();
    }
  }
  const progress = connection.progress.data;
  const retryAsset =
    progress?.selected_asset_id &&
    (progress.status === "installing" ||
      (progress.status === "completed" &&
        progress.subscription_state === "pending"))
      ? progress.selected_asset_id
      : null;
  return (
    <div className="space-y-4">
      <p className="type-body text-muted-foreground">
        {t(
          "Connect a Facebook Page or an Instagram Business or Creator account. Customers can start private text conversations; replies must stay within the provider's 24-hour window.",
        )}
      </p>
      <Button
        type="button"
        variant="primary"
        disabled={
          bootstrap?.available === false ||
          connection.start.isPending ||
          connection.finish.isPending
        }
        onClick={() => void start()}
      >
        {botId ? t("Reconnect inbox") : t("Sign in and grant inbox access")}
      </Button>
      {progress?.status === "pending" && (
        <Button
          type="button"
          disabled={connection.start.isPending}
          onClick={() => {
            popup.current?.close();
            setSessionId(null);
          }}
        >
          {t("Cancel sign-in")}
        </Button>
      )}
      {progress?.status === "completed" &&
        progress.subscription_state === "suspended" && (
          <p role="status">
            {t("Inbox access was restored. Resume delivery to complete setup.")}
          </p>
        )}
      {progress?.status === "completed" &&
        (progress.subscription_state === "active" ||
          progress.subscription_state === "suspended") &&
        progress.bot_id && (
          <Link
            className="type-row-title underline"
            to="/channel-bots/$botId"
            params={{ botId: progress.bot_id }}
          >
            {t("Open connected inbox")}
          </Link>
        )}
      {retryAsset && (
        <div className="space-y-2">
          <p role="status">
            {t("Subscription verification pending for inbox {{id}}.", {
              id: retryAsset,
            })}
          </p>
          <Button
            type="button"
            variant="primary"
            disabled={connection.start.isPending || connection.finish.isPending}
            onClick={() => void finish(retryAsset, true)}
          >
            {t("Retry verification")}
          </Button>
        </div>
      )}
      {progress?.status === "assets" && (
        <fieldset className="space-y-3" disabled={connection.start.isPending || connection.finish.isPending}>
          <Label asChild>
            <legend>{t("Select the inbox to connect")}</legend>
          </Label>
          {progress.assets.map((asset) => (
            <Label key={asset.id} className="flex items-center gap-2">
              <input
                type="radio"
                name="meta-asset"
                value={asset.id}
                checked={selected === asset.id}
                onChange={() => {
                  setSelected(asset.id);
                  setExclusive(false);
                }}
              />
              {asset.name}{" "}
              <span className="text-text-tertiary">{asset.id}</span>
            </Label>
          ))}
          <Label className="flex items-start gap-2">
            <Checkbox
              checked={exclusive}
              onCheckedChange={(value) => setExclusive(value === true)}
            />
            {t(
              "I have verified that this app receives this inbox's conversations exclusively. Other inbox apps and unsupported coexistence modes must be removed before activation.",
            )}
          </Label>
          <Button
            type="button"
            variant="primary"
            disabled={!selected || !exclusive || connection.finish.isPending}
            onClick={() => void finish(selected, exclusive)}
          >
            {t("Connect selected inbox")}
          </Button>
        </fieldset>
      )}
      {progress?.status === "cancelled" && (
        <p role="status" className="type-body text-muted-foreground">
          {t(
            "Sign-in was cancelled. No inbox was connected. Return to setup to reconnect.",
          )}
        </p>
      )}
      {progress?.status === "revoked" && (
        <p role="status" className="type-body text-muted-foreground">
          {t("Inbox consent was revoked. Return to setup and reconnect.")}
        </p>
      )}
      {progress &&
        ["failed", "no_eligible_assets"].includes(progress.status) && (
          <p role="status" className="type-body text-muted-foreground">
            {t(
              "Sign-in did not produce an eligible inbox. Check your Page messaging task or Instagram professional account, then reconnect.",
            )}
          </p>
        )}
      {popupClosed && progress?.status === "pending" && (
        <ErrorBanner
          message={t(
            "The sign-in window closed. Reconnect if you did not finish consent.",
          )}
        />
      )}
      {error && <ErrorBanner message={error} />}
      {connection.progress.isError && (
        <ErrorBanner message={connection.progress.error.message} />
      )}
      <ol className="list-decimal space-y-2 pl-5 type-caption text-muted-foreground">
        <li>
          {t("Grant provider consent and select the verified account above.")}
        </li>
        <li>
          {t("Confirm webhook health with Verify on the bot details page.")}
        </li>
        <li>
          {t(
            "Assign a support specialist. Review its guest model and tool grants before choosing Everyone for private chats. Organization inboxes also require guest exposure and a funded organization wallet.",
          )}
        </li>
        <li>
          {t(
            "Send a message from an unrelated test account in the native inbox and check its reply. The first customer message never verifies the inbox owner.",
          )}
        </li>
      </ol>
    </div>
  );
}

export function ManagedMetaDetail({ bot, orgId }: ManagedDetailProps) {
  const { t } = useUiTranslation();
  const client = useQueryClient();
  return (
    <div className="space-y-4">
      <ChannelBotHealthCard botId={bot.id} />
      <MetaCustomerSetup botId={bot.id} orgId={orgId} />
      <details>
        <summary className="type-row-title cursor-pointer">
          {t("Reconnect inbox")}
        </summary>
        <ManagedMetaConnect
          platform={bot.platform}
          label={bot.label}
          orgId={orgId}
          botId={bot.id}
          onConnected={() => {
            void client.invalidateQueries({ queryKey: ["channel-bots"] });
          }}
        />
      </details>
      <p className="type-caption text-muted-foreground">
        {t(
          "Disconnect disables local delivery first and removes only this inbox's subscription. Other Pages connected by the same administrator remain connected.",
        )}
      </p>
    </div>
  );
}
