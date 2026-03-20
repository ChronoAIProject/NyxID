import { useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { AlertCircle } from "lucide-react";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useProvider, useTelegramConnectConfig } from "@/hooks/use-providers";
import { ApiError } from "@/lib/api-client";
import { normalizeTelegramBotUsername } from "@/lib/telegram-login";

const TELEGRAM_WIDGET_SRC = "https://telegram.org/js/telegram-widget.js?22";

export function ProviderTelegramConnectPage() {
  const { providerId } = useParams({ strict: false }) as {
    providerId: string;
  };
  const navigate = useNavigate();
  const { data: provider, isLoading: providerLoading, error: providerError } =
    useProvider(providerId);
  const {
    data: config,
    isLoading: configLoading,
    error: configError,
  } = useTelegramConnectConfig(providerId);
  const widgetRef = useRef<HTMLDivElement | null>(null);
  const [widgetError, setWidgetError] = useState<string | null>(null);

  useEffect(() => {
    if (!config || widgetRef.current === null) {
      return;
    }

    const container = widgetRef.current;
    container.innerHTML = "";
    setWidgetError(null);

    const script = document.createElement("script");
    script.async = true;
    script.src = TELEGRAM_WIDGET_SRC;
    script.setAttribute(
      "data-telegram-login",
      normalizeTelegramBotUsername(config.bot_username),
    );
    script.setAttribute("data-size", "large");
    script.setAttribute("data-radius", "10");
    script.setAttribute("data-auth-url", config.redirect_url);
    script.setAttribute("data-request-access", "write");
    script.onerror = () => {
      setWidgetError("Failed to load the Telegram Login Widget.");
    };

    container.appendChild(script);

    return () => {
      container.innerHTML = "";
    };
  }, [config]);

  const isLoading = providerLoading || configLoading;
  const errorMessage =
    widgetError ??
    (configError instanceof ApiError
      ? configError.message
      : providerError
        ? "Unable to load this provider."
        : null);

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (errorMessage || !provider || !config) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Telegram connection unavailable
        </h3>
        <p className="mb-4 max-w-md text-sm text-muted-foreground">
          {errorMessage ?? "This Telegram provider is not ready to connect."}
        </p>
        <Button variant="outline" onClick={() => void navigate({ to: "/providers" })}>
          Back to Providers
        </Button>
      </div>
    );
  }

  if (provider.provider_type !== "telegram_widget") {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Unsupported provider
        </h3>
        <p className="mb-4 max-w-md text-sm text-muted-foreground">
          This connection page only supports Telegram Login Widget providers.
        </p>
        <Button variant="outline" onClick={() => void navigate({ to: "/providers" })}>
          Back to Providers
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Providers", to: "/providers" },
          { label: provider.name },
          { label: "Connect" },
        ]}
        title={`Connect ${provider.name}`}
        description="Verify your Telegram identity with the Telegram Login Widget."
        actions={
          <Button
            variant="outline"
            onClick={() => void navigate({ to: "/providers" })}
          >
            Cancel
          </Button>
        }
      />

      <Card className="mx-auto max-w-2xl">
        <CardHeader className="text-center">
          <CardTitle>Continue in Telegram</CardTitle>
          <CardDescription>
            Approve the login request to connect your Telegram account to NyxID.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col items-center gap-4">
          <div
            ref={widgetRef}
            className="flex min-h-14 items-center justify-center"
          />
          <p className="text-center text-sm text-muted-foreground">
            Telegram will return you to NyxID after you confirm the login.
          </p>
          <p className="text-center text-xs text-muted-foreground/70">
            Widget bot: @{normalizeTelegramBotUsername(config.bot_username)}
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
