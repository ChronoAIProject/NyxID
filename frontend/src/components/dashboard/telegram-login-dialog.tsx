import { useEffect, useRef, useCallback, useMemo } from "react";
import type { ProviderConfig } from "@/types/api";
import type { TelegramLoginData } from "@/types/api";
import {
  useTelegramWidgetConfig,
  useConnectTelegramWidget,
} from "@/hooks/use-providers";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { CheckCircle2, AlertCircle, Loader2 } from "lucide-react";
import { ApiError } from "@/lib/api-client";

declare global {
  interface Window {
    onTelegramAuth?: (user: TelegramLoginData) => void;
  }
}

type FlowStep = "loading" | "ready" | "submitting" | "success" | "error";

interface TelegramLoginDialogProps {
  readonly provider: ProviderConfig;
  readonly onClose: () => void;
}

export function TelegramLoginDialog({
  provider,
  onClose,
}: TelegramLoginDialogProps) {
  const widgetContainerRef = useRef<HTMLDivElement>(null);
  const scriptRef = useRef<HTMLScriptElement | null>(null);

  const { data: config, error: configError } = useTelegramWidgetConfig(
    provider.id,
  );
  const connectMutation = useConnectTelegramWidget();

  // Derive step from query/mutation state instead of useState
  const step: FlowStep = useMemo(() => {
    if (configError || connectMutation.isError) return "error";
    if (connectMutation.isSuccess) return "success";
    if (connectMutation.isPending) return "submitting";
    if (config) return "ready";
    return "loading";
  }, [configError, connectMutation.isError, connectMutation.isSuccess, connectMutation.isPending, config]);

  const errorMessage = useMemo(() => {
    if (configError) {
      return configError instanceof ApiError
        ? configError.message
        : "Failed to load Telegram login configuration";
    }
    if (connectMutation.error) {
      return connectMutation.error instanceof ApiError
        ? connectMutation.error.message
        : "Failed to verify Telegram login";
    }
    return "";
  }, [configError, connectMutation.error]);

  const handleTelegramAuth = useCallback(
    (user: TelegramLoginData) => {
      connectMutation.mutate({ providerId: provider.id, data: user });
    },
    [connectMutation, provider.id],
  );

  useEffect(() => {
    if (!config || !widgetContainerRef.current) return;

    // Set up global callback for the Telegram widget
    window.onTelegramAuth = handleTelegramAuth;

    // Clear previous widget content
    const container = widgetContainerRef.current;
    container.innerHTML = "";

    // Create and append the Telegram Login Widget script
    const script = document.createElement("script");
    script.src = "https://telegram.org/js/telegram-widget.js?22";
    script.async = true;
    script.setAttribute("data-telegram-login", config.bot_username);
    script.setAttribute("data-size", "large");
    script.setAttribute("data-onauth", "onTelegramAuth(user)");
    container.appendChild(script);
    scriptRef.current = script;

    return () => {
      delete window.onTelegramAuth;
      if (scriptRef.current?.parentNode) {
        scriptRef.current.parentNode.removeChild(scriptRef.current);
        scriptRef.current = null;
      }
    };
  }, [config, handleTelegramAuth]);

  return (
    <Dialog open onOpenChange={onClose}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Login with {provider.name}</DialogTitle>
          <DialogDescription>
            {step === "loading" && "Loading Telegram login..."}
            {step === "ready" &&
              "Click the Telegram button below to verify your identity."}
            {step === "submitting" && "Verifying your Telegram identity..."}
            {step === "success" &&
              `Successfully connected to ${provider.name}.`}
            {step === "error" && "An error occurred during the connection."}
          </DialogDescription>
        </DialogHeader>

        {step === "loading" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <p className="text-sm text-muted-foreground">
              Loading Telegram login widget...
            </p>
          </div>
        )}

        {step === "ready" && (
          <div className="space-y-4">
            <div className="flex flex-col items-center gap-4 py-4">
              <div ref={widgetContainerRef} className="flex justify-center" />
              <p className="text-xs text-muted-foreground text-center">
                A Telegram popup will open. Sign in and confirm to connect your
                account.
              </p>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === "submitting" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <p className="text-sm text-muted-foreground">
              Verifying your Telegram identity...
            </p>
          </div>
        )}

        {step === "success" && (
          <div className="space-y-4">
            <div className="flex flex-col items-center gap-3 py-4">
              <CheckCircle2 className="h-10 w-10 text-success" />
              <p className="text-sm text-muted-foreground text-center">
                Your Telegram account has been connected successfully.
              </p>
            </div>
            <DialogFooter>
              <Button type="button" onClick={onClose}>
                Done
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === "error" && (
          <div className="space-y-4">
            <div className="flex flex-col items-center gap-3 py-4">
              <AlertCircle className="h-10 w-10 text-destructive" />
              <p className="text-sm text-destructive text-center">
                {errorMessage}
              </p>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Close
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
