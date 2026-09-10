import { useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  ArrowLeft,
  Eye,
  EyeOff,
  ExternalLink,
  LockKeyhole,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { useRegisterNyxbotTelegram } from "@/hooks/use-nyxbot-channels";
import { useManagedOnboarding } from "@/hooks/use-channel-managed";
import {
  createNyxbotTelegramSchema,
  type NyxbotTelegramForm,
  type NyxbotChannel,
} from "@/schemas/nyxbot-onboarding";
import {
  NyxbotChannelError,
  type NyxbotChannelRegistration,
} from "@/lib/nyxbot-channels";
import {
  OnboardingNotice,
  OnboardingShell,
  StepHelp,
} from "./onboarding-shell";
import { BrandIcon } from "./brand-icon";
import { AEVATAR_CHANNELS_URL } from "@/lib/nyxbot-aevatar-auth";

const CHANNEL_AVAILABILITY: Record<NyxbotChannel, boolean> = {
  telegram: true,
  whatsapp: false,
};

export function ChannelStep({
  channel: preferredChannel,
  referral,
  onSelect,
  onBack,
  onConnected,
  onSpendingCap,
}: {
  readonly channel: NyxbotChannel | null;
  readonly referral?: NyxbotChannel;
  readonly onSelect: (channel: NyxbotChannel) => void;
  readonly onBack: () => void;
  readonly onConnected: (registration: NyxbotChannelRegistration) => void;
  readonly onSpendingCap: () => void;
}) {
  const { t } = useTranslation();
  const channel =
    preferredChannel && CHANNEL_AVAILABILITY[preferredChannel]
      ? preferredChannel
      : null;
  const [showToken, setShowToken] = useState(false);
  const [needsConsent, setNeedsConsent] = useState(false);
  const submitting = useRef(false);
  const createBot = useRegisterNyxbotTelegram();
  const managed = useManagedOnboarding("whatsapp", channel === "whatsapp");
  const telegramSchema = createNyxbotTelegramSchema({
    required: t("tokenRequired"),
    invalid: t("tokenInvalid"),
  });
  const form = useAppForm<NyxbotTelegramForm>({
    resolver: zodResolver(telegramSchema),
    defaultValues: { bot_token: "" },
    mode: "onChange",
  });
  async function submit(values: NyxbotTelegramForm) {
    if (submitting.current || channel !== "telegram") return;
    submitting.current = true;
    setNeedsConsent(false);
    try {
      const result = await createBot.mutateAsync(values.bot_token);
      form.reset();
      createBot.reset();
      onConnected(result);
    } catch (error) {
      setNeedsConsent(
        error instanceof NyxbotChannelError &&
          error.code === "channelConsentRequired",
      );
      // Do not echo provider errors that might contain submitted credentials.
      form.setError("bot_token", {
        message: t(
          error instanceof NyxbotChannelError ? error.code : "channelFailed",
        ),
      });
    } finally {
      submitting.current = false;
    }
  }
  return (
    <OnboardingShell
      title={t("channelTitle")}
      subtitle={t("channelIntro")}
      step={3}
      variant="setup"
      actions={
        <>
          <Button
            type="button"
            className="nb-secondary"
            onClick={onBack}
            disabled={createBot.isPending}
          >
            <ArrowLeft size={18} aria-hidden="true" />
            {t("back")}
          </Button>
          {channel === "whatsapp" ? (
            <Button
              type="button"
              className="nb-primary"
              onClick={onSpendingCap}
            >
              {t("reviewCap")}
            </Button>
          ) : (
            <Button
              form="nyxbot-telegram"
              type="submit"
              className="nb-primary"
              isLoading={createBot.isPending}
              disabled={
                channel !== "telegram" ||
                !telegramSchema.safeParse(form.watch()).success ||
                createBot.isPending
              }
            >
              {t("connectChannel")}
            </Button>
          )}
        </>
      }
    >
      <p className="nb-channel-conversation">
        <BrandIcon brand="telegram" />
        {t("channelConversation")}
      </p>
      <fieldset
        className="nb-tiles nb-channel-tiles"
        disabled={createBot.isPending}
      >
        <legend className="sr-only">{t("channelTitle")}</legend>
        {(["telegram", "whatsapp"] as const).map((option) => (
          <label
            className={
              CHANNEL_AVAILABILITY[option]
                ? "nb-tile"
                : "nb-tile nb-tile-unavailable"
            }
            data-selected={channel === option}
            key={option}
          >
            <input
              type="radio"
              name="channel"
              value={option}
              disabled={!CHANNEL_AVAILABILITY[option]}
              checked={channel === option}
              onChange={() => {
                form.reset();
                setShowToken(false);
                setNeedsConsent(false);
                onSelect(option);
              }}
            />
            <BrandIcon brand={option} />
            <strong>{t(option)}</strong>
            <span>
              {t(
                !CHANNEL_AVAILABILITY[option]
                  ? "comingSoon"
                  : channel === option
                    ? referral === option
                      ? "preselected"
                      : "selected"
                    : "available",
              )}
            </span>
          </label>
        ))}
        {[
          { id: "instagram", name: "Instagram" },
          { id: "facebook", name: "FB Messenger" },
        ].map((option) => (
          <div
            className="nb-tile nb-tile-unavailable"
            aria-disabled="true"
            key={option.id}
          >
            <BrandIcon brand={option.id} />
            <div className="nb-channel-unavailable-copy">
              <strong>{option.name}</strong>
              <span>{t("comingSoon")}</span>
            </div>
          </div>
        ))}
      </fieldset>
      {channel === "telegram" && (
        <>
          <ol className="nb-instructions">
            <li>
              <a
                href="https://t.me/BotFather"
                target="_blank"
                rel="noopener noreferrer"
              >
                {t("telegramOpen")}
              </a>{" "}
              {t("telegramCommand")}
            </li>
            <li>{t("telegramName")}</li>
            <li>{t("telegramPaste")}</li>
          </ol>
          <Form {...form}>
            <form
              id="nyxbot-telegram"
              className="nb-channel-form"
              onSubmit={form.handleSubmit(submit)}
            >
              <FormField
                control={form.control}
                name="bot_token"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("token")}</FormLabel>
                    <div className="nb-token-field">
                      <FormControl>
                        <Input
                          {...field}
                          disabled={createBot.isPending}
                          type={showToken ? "text" : "password"}
                          placeholder={t("tokenPlaceholder")}
                          autoComplete="off"
                          spellCheck={false}
                          autoCapitalize="none"
                        />
                      </FormControl>
                      <button
                        type="button"
                        disabled={createBot.isPending}
                        aria-label={t(showToken ? "hideToken" : "showToken")}
                        title={t(showToken ? "hideToken" : "showToken")}
                        onClick={() => setShowToken((shown) => !shown)}
                      >
                        {showToken ? <EyeOff size={16} /> : <Eye size={16} />}
                      </button>
                    </div>
                    <FormMessage role="alert" />
                  </FormItem>
                )}
              />
            </form>
          </Form>
          {needsConsent && (
            <p className="nb-small">
              <a
                href={AEVATAR_CHANNELS_URL}
                target="_blank"
                rel="noopener noreferrer"
              >
                {t("authorizeAevatar")} <ExternalLink size={12} />
              </a>
            </p>
          )}
          <p className="nb-channel-token-note">
            <LockKeyhole size={16} aria-hidden="true" />
            {t("tokenNote")}
          </p>
        </>
      )}
      {channel === "whatsapp" && (
        <>
          <p className="nb-intro">
            {t("whatsappIntro")} <StepHelp>{t("whatsappHelp")}</StepHelp>
          </p>
          <ol className="nb-instructions">
            <li>{t("whatsappLogin")}</li>
            <li>{t("whatsappNumber")}</li>
            <li>{t("whatsappVerify")}</li>
          </ol>
          <OnboardingNotice>{t("capUnavailable")}</OnboardingNotice>
          {managed.isPending && (
            <p role="status" className="nb-small">
              {t("loading")}
            </p>
          )}
          {(managed.isError || (managed.data && !managed.data.available)) && (
            <OnboardingNotice>{t("whatsappUnavailable")}</OnboardingNotice>
          )}
          <Button type="button" className="nb-secondary nb-full" disabled>
            <ExternalLink size={14} />
            {t("connectWhatsapp")}
          </Button>
        </>
      )}
    </OnboardingShell>
  );
}
