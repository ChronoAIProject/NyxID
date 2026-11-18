import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useRouterState,
} from "@tanstack/react-router";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { ChannelConnectionShell } from "@/components/channels/channel-connection-shell";
import { useAuthStore } from "@/stores/auth-store";
import { ChannelBotSetup } from "@/components/channels/channel-bot-setup";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { CopyableUrlCallout } from "@/components/shared/copyable-url-callout";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useChannelPlatformViews } from "@/hooks/use-channel-platforms";
import type {
  ChannelPlatform,
  ChannelPlatformDescriptor,
} from "@/types/channels";
import {
  channelBotSetupPrefill,
  channelBotSetupUrlValues,
  parseRawChannelBotReturnTo,
} from "@/schemas/channel-bot-setup";
import { labelIsOptional } from "@/schemas/channels";

function setupUrl(platform: ChannelPlatform) {
  return `${window.location.origin}/channel-bots/connect/${encodeURIComponent(platform)}`;
}

function SetupLoading() {
  useUiTranslation();
  return (
    <div role="status" className="space-y-4">
      <span className="sr-only">{translateUi("Loading channel setup...")}</span>
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-64 w-full rounded-xl" />
    </div>
  );
}

export function ChannelBotSetupLinksPage() {
  useUiTranslation();
  const catalog = useChannelPlatformViews();
  const platforms =
    catalog.data?.platforms.filter((platform) => platform.enabled) ?? [];
  useBreadcrumbLabel("Setup links");

  return (
    <div className="space-y-8">
      <PageHeader
        title={translateUi("Channel setup links")}
        description={translateUi("Open a channel's setup page or share its link as part of bot onboarding.")}
        actions={
          <Button variant="outline" asChild>
            <Link to="/channel-bots">
              <ArrowLeft aria-hidden="true" />{translateUi("Channel Bots")}</Link>
          </Button>
        }
      />
      {catalog.isPending ? (
        <SetupLoading />
      ) : catalog.isError ? (
        <ErrorBanner
          message={translateUi("Unable to load supported channels.")}
          onRetry={catalog.refetch}
        />
      ) : platforms.length === 0 ? (
        <p className="text-13 text-muted-foreground">{translateUi("No channels are currently available for setup.")}</p>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {platforms.map((platform) => (
            <section
              key={platform.platform}
              className="flex min-w-0 flex-col gap-4 rounded-xl border border-border/50 bg-card p-4"
            >
              <h3 className="type-card-title">
                {platform.display_name}
              </h3>
              <CopyableUrlCallout
                label={translateUi("{{value1}} setup link", { value1: platform.display_name })}
                url={setupUrl(platform.platform)}
              />
              <Button variant="outline" asChild className="self-start">
                <Link
                  to="/channel-bots/connect/$platform"
                  params={{ platform: platform.platform }}
                >{translateUi("Set up ")}{platform.display_name}
                  <ArrowRight aria-hidden="true" />
                </Link>
              </Button>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

export function ChannelBotSetupPage() {
  useUiTranslation();
  const { platform } = useParams({
    from: "/channel-bots/connect/$platform",
  });
  const isAuthenticated = useAuthStore((state) => state.isAuthenticated);
  if (!isAuthenticated) return null;
  return <ChannelSetupConnection key={platform} platform={platform} />;
}

function ChannelSetupConnection({ platform }: { readonly platform: string }) {
  useUiTranslation();
  const catalog = useChannelPlatformViews();
  const email = useAuthStore((state) => state.user?.email);
  const [complete, setComplete] = useState(false);
  const [returnTo] = useState(() => parseRawChannelBotReturnTo(window.location.search));
  const descriptor = catalog.data?.platforms.find(
    (entry) => entry.platform === platform,
  );
  const platformName = descriptor
    ? platform === "telegram" || platform === "telegram-new"
      ? "Telegram"
      : descriptor.display_name
    : undefined;

  return (
    <ChannelConnectionShell
      platform={platform}
      platformName={platformName}
      complete={complete}
      email={email}
      guideUrl={
        descriptor?.enabled
          ? descriptor.registration.documentation_url
          : undefined
      }
    >
      {returnTo && !complete ? (
        <Button variant="outline" asChild>
          <Link to="/assistant" search={returnTo.startsWith("nyxg-") ? { g: returnTo } : { c: returnTo }}>{translateUi("Back to chat")}</Link>
        </Button>
      ) : null}
      {catalog.isPending ? (
        <SetupLoading />
      ) : catalog.isError ? (
        <ErrorBanner
          message={translateUi("Unable to load channel setup.")}
          onRetry={catalog.refetch}
        />
      ) : !descriptor?.enabled ? (
        <div className="space-y-4 text-center">
          <p role="status" className="text-13 text-muted-foreground">
            {descriptor
              ? translateUi("{{value1}} setup is currently unavailable.", { value1: descriptor.display_name })
              : translateUi("This channel is not supported.")}
          </p>
          <Button variant="outline" asChild>
            <Link to="/channel-bots/connect">{translateUi("Choose another channel")}</Link>
          </Button>
        </div>
      ) : (
        <section
          aria-label={translateUi("{{value1}} setup", { value1: descriptor.display_name })}
          className="space-y-5"
        >
          <PrefilledChannelBotSetup
            descriptor={descriptor}
            returnTo={returnTo}
            onComplete={() => setComplete(true)}
          />
        </section>
      )}
    </ChannelConnectionShell>
  );
}

function PrefilledChannelBotSetup({
  descriptor,
  returnTo,
  onComplete,
}: {
  readonly descriptor: ChannelPlatformDescriptor;
  readonly returnTo?: string;
  readonly onComplete: () => void;
}) {
  useUiTranslation();
  const searchStr = useRouterState({
    select: (state) => state.location.searchStr,
  });
  const search = channelBotSetupUrlValues(searchStr);
  const navigate = useNavigate();
  const [prefill] = useState(() =>
    channelBotSetupPrefill(searchStr, descriptor),
  );

  useEffect(() => {
    const values = channelBotSetupUrlValues(searchStr);
    const secrets = descriptor.registration.fields.filter(
      (field) => field.secret && Object.hasOwn(values, field.name),
    );
    if (!secrets.length) return;
    for (const field of secrets) delete values[field.name];
    void navigate({
      to: "/channel-bots/connect/$platform",
      params: { platform: descriptor.platform },
      search: values,
      replace: true,
    });
  }, [descriptor, searchStr, navigate]);

  return (
    <ChannelBotSetup
      fullPage
      onComplete={onComplete}
      returnTo={returnTo}
      onContinue={returnTo ? () => {
        if (returnTo.startsWith("nyxg-")) {
          return navigate({ to: "/assistant", search: { g: returnTo } });
        }
        return navigate({ to: "/assistant", search: { c: returnTo } });
      } : undefined}
      open
      defaultPlatform={descriptor.platform}
      defaultLabel={
        search.label?.trim()
          ? search.label.slice(0, 128)
          : labelIsOptional(descriptor.platform)
            ? ""
            : translateUi("{{value1}} bot", { value1: descriptor.display_name })
      }
      defaultOrgId={search.target_org_id ?? null}
      prefill={prefill}
      onOpenChange={(open) => {
        if (!open) void navigate({ to: "/channel-bots" });
      }}
    />
  );
}
