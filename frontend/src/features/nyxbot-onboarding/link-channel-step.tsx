import { Copy, ExternalLink, ShieldCheck, CircleHelp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useChannelBot } from "@/hooks/use-channel-bots";
import { useNyxbotRegistrationStatus } from "@/hooks/use-nyxbot-channels";
import { AEVATAR_WEBHOOK_BASE_URL } from "@/lib/nyxbot-channels";
import {
  OnboardingShell,
  OnboardingNotice,
  StepHelp,
  BackButton,
} from "./onboarding-shell";

export function LinkChannelStep({
  botId,
  registrationId,
  onBack,
}: {
  readonly botId: string;
  readonly registrationId: string | null;
  readonly onBack: (missing?: boolean) => void;
}) {
  const { t } = useTranslation();
  const bot = useChannelBot(botId);
  const registration = useNyxbotRegistrationStatus(registrationId);
  const registrationReady =
    !registrationId ||
    (registration.data?.registration_id === registrationId &&
      registration.data.nyx_channel_bot_id === botId &&
      registration.data.status === "active");
  const channelName =
    bot.data?.platform === "whatsapp" ? "WhatsApp" : "Telegram";
  const ready =
    bot.data?.status === "active" &&
    bot.data.is_active &&
    bot.data.webhook_registered &&
    registrationReady;
  return (
    <OnboardingShell
      title={t("linkTitle")}
      subtitle={t("linkSubtitle")}
      step={3}
      actions={
        <>
          <Button className="nb-primary" disabled>
            <ExternalLink size={14} />
            {t("openChat", { channel: channelName })}
          </Button>
          <Button className="nb-secondary" disabled>
            <Copy size={14} />
            {t("copyCode")}
          </Button>
        </>
      }
    >
      <p className="nb-next">{t("linkNext", { channel: channelName })}</p>
      <p className="nb-intro">
        {t("linkIntro")} <StepHelp>{t("linkHelp")}</StepHelp>
      </p>
      <ul className="nb-link-reasons">
        <li>
          <ShieldCheck size={14} />
          {t("linkReason")}
        </li>
        <li>
          <CircleHelp size={14} />
          {t("linkThen")}
        </li>
      </ul>
      <label className="nb-code-label" htmlFor="nyxbot-pairing-code">
        {t("code")}
      </label>
      <Input id="nyxbot-pairing-code" placeholder={t("codePending")} disabled />
      {bot.isPending || (registrationId && registration.isPending) ? (
        <OnboardingNotice>{t("loading")}</OnboardingNotice>
      ) : bot.isError || (registrationId && registration.isError) ? (
        <OnboardingNotice error>
          {t("loadError")}{" "}
          <Button
            onClick={() => {
              void bot.refetch();
              if (registrationId) void registration.refetch();
            }}
          >
            {t("retry")}
          </Button>
        </OnboardingNotice>
      ) : (
        <OnboardingNotice>
          {t(
            ready
              ? "linkUnavailable"
              : registrationId
                ? "channelRegistrationPending"
                : "channelPending",
          )}
          {!ready && registrationId && (
            <Button
              onClick={() => {
                void bot.refetch();
                void registration.refetch();
              }}
            >
              {t("retry")}
            </Button>
          )}
        </OnboardingNotice>
      )}
      <div className="nb-inline-actions">
        {bot.data && (
          <Button className="nb-secondary" asChild>
            <a
              href={
                registrationId
                  ? `${AEVATAR_WEBHOOK_BASE_URL}/channels`
                  : `/channel-bots/${encodeURIComponent(botId)}`
              }
              target={registrationId ? "_blank" : undefined}
              rel={registrationId ? "noopener noreferrer" : undefined}
            >
              <ExternalLink size={14} />
              {t("manageChannel")}
            </a>
          </Button>
        )}
        <BackButton
          onClick={() =>
            onBack(
              Boolean(
                bot.error && "status" in bot.error && bot.error.status === 404,
              ),
            )
          }
        />
      </div>
    </OnboardingShell>
  );
}
