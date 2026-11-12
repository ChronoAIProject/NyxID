import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import type { ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ServiceIcon } from "@/components/service-icon";
import { TelegramGlyph } from "@/components/service-icons/_shared";
import { ConnectionArc } from "@/components/shared/connection-arc";
import { useApplyTheme } from "@/hooks/use-theme";

function ChannelIcon({ platform }: { readonly platform: string }) {
  useUiTranslation();
  if (platform === "telegram" || platform === "telegram-new") {
    return <TelegramGlyph className="size-9 text-foreground" />;
  }
  if (platform === "whatsapp") {
    return <img src="/nyxbot-whatsapp.svg" alt="" className="size-9 dark:invert" />;
  }
  return (
    <ServiceIcon
      slug={`api-${platform === "x" ? "twitter" : platform}`}
      size="xl"
      className="text-foreground"
    />
  );
}

export function ChannelConnectionShell({
  platform,
  platformName,
  complete,
  email,
  guideUrl,
  children,
}: {
  readonly platform: string;
  readonly platformName?: string;
  readonly complete: boolean;
  readonly email?: string;
  readonly guideUrl?: string | null;
  readonly children: ReactNode;
}) {
  useUiTranslation();
  useApplyTheme();
  return (
    <div className="flex min-h-dvh flex-col bg-background px-4 py-6 text-foreground sm:py-10">
      <div className="m-auto w-full max-w-[520px]">
        <main className="channel-connection-card relative overflow-hidden rounded-xl border border-border bg-card">
          <div className="px-5 py-8 sm:px-8">
            {guideUrl && (
              <div className="mb-6 flex justify-end">
                <a
                  href={guideUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1.5 type-row-title text-muted-foreground hover:text-foreground"
                >{translateUi("Channel setup guide")}<ExternalLink className="size-3" aria-hidden="true" />
                </a>
              </div>
            )}
            <div
              className="mb-6 flex items-start justify-center"
              role="img"
              aria-label={translateUi("NyxID connects to {{value1}}", { value1: platformName ?? translateUi("your channel") })}
            >
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                  <NyxidIcon className="size-9" alt="" />
                </div>
                <span className="type-row-title">NyxID</span>
              </div>
              <ConnectionArc complete={complete} />
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                  <ChannelIcon platform={platform} />
                </div>
                <span className="text-center type-row-title">
                  {platformName ?? translateUi("Channel")}
                </span>
              </div>
            </div>
            <div className="mb-7 space-y-2 text-center">
              <h1 className="font-display type-compact-page-title tracking-tight sm:type-page-title">
                {platform === "telegram"
                  ? complete
                    ? translateUi("Your Telegram bot is connected")
                    : translateUi("Connect your Telegram bot")
                  : platformName
                    ? complete
                      ? translateUi("Your {{value1}} channel bot is created", { value1: platformName })
                      : translateUi("Create your {{value1}} channel bot", { value1: platformName })
                    : translateUi("Set up your channel bot")}
              </h1>
              <p className="text-14 text-muted-foreground">
                {complete
                  ? translateUi("Finish any platform setup, then choose an agent to respond to your messages.")
                  : platform === "telegram-new"
                    ? translateUi("Create a bot in Telegram and connect it to your AI agents through NyxID.")
                    : translateUi("Connect your {{value1}} bot to NyxID so your AI agents can receive and reply to messages.", { value1: platformName ?? "channel" })}
              </p>
            </div>
            {children}
          </div>
          {email && (
            <div className="border-t border-border/60 px-5 py-3 text-center text-13 text-muted-foreground sm:px-8">{translateUi("Signed in as")}{" "}
              <span className="break-all text-foreground">{email}</span>
            </div>
          )}
        </main>
        <footer className="mt-6 flex h-7 items-center justify-center text-center text-13 text-muted-foreground">{translateUi("Channel connections by NyxID")}</footer>
      </div>
    </div>
  );
}
