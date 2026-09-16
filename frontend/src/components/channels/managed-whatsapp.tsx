import { useEffect, useRef, useState } from "react";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { useReregisterChannelBot, useRepairChannelBot } from "@/hooks/use-channel-managed";
import type { ChannelBotDetail } from "@/types/channels";
import { useQueryClient } from "@tanstack/react-query";
import { ExternalLink, Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  loadFacebookSdk,
  parseEmbeddedSignupEvent,
  type FacebookSdk,
} from "@/lib/meta-embedded-signup";
import { completeManagedOnboarding } from "@/hooks/use-channel-managed";
import { useRuntimeConfig } from "@/hooks/use-runtime-config";
import type { ManagedBootstrap } from "@/schemas/channel-managed";
import type { CreateChannelBotResponse } from "@/types/channels";

const stageLabels: Record<string, string> = {
  exchanging: "Exchanging...",
  subscribing: "Subscribing...",
  registering: "Registering...",
};

export function ManagedWhatsAppDetail({ bot }: { readonly bot: ChannelBotDetail }) {
  const reregister = useReregisterChannelBot();
  const repair = useRepairChannelBot();
  return <DetailSection title="Managed setup">
    <DetailRow label="Subscription" value={bot.managed_setup?.subscription ?? "pending"} />
    <DetailRow label="Webhook override" value={bot.managed_setup?.webhook_override ?? "pending"} />
    <DetailRow label="Number registration" value={bot.managed_setup?.registration ?? "pending"} />
    {Object.entries(bot.managed_setup?.coexistence_sync ?? {}).map(([name, status]) => <DetailRow key={name} label={name === "history" ? "History sync" : "Contact sync"} value={status} />)}
    <div className="flex flex-wrap gap-2 p-4">
      <Button variant="outline" disabled={repair.isPending} isLoading={reregister.isPending} onClick={() => reregister.mutate(bot.id, { onSuccess: () => toast.success("Number registration checked"), onError: (error) => toast.error(error instanceof ApiError ? error.message : "Unable to re-register number") })}><ShieldCheck className="size-3" />Re-register number</Button>
      <Button variant="outline" disabled={reregister.isPending} isLoading={repair.isPending} onClick={() => repair.mutate(bot.id, { onSuccess: () => toast.success("Setup checked"), onError: (error) => toast.error(error instanceof ApiError ? error.message : "Unable to repair setup") })}><ShieldCheck className="size-3" />Repair setup</Button>
    </div>
  </DetailSection>;
}

export function ManagedWhatsApp({
  bootstrap,
  label,
  orgId,
  onConnected,
}: {
  readonly bootstrap: ManagedBootstrap;
  readonly label: string;
  readonly orgId: string | null;
  readonly onConnected: (bot: CreateChannelBotResponse) => void;
}) {
  const [sdk, setSdk] = useState<FacebookSdk | null>(null);
  const [sdkAttempt, setSdkAttempt] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [feature, setFeature] = useState("");
  const [stage, setStage] = useState<string | null>(null);
  const [popupHint, setPopupHint] = useState(false);
  const cleanupRef = useRef<(() => void) | null>(null);
  const busyRef = useRef(false);
  const client = useQueryClient();
  const { data: runtimeConfig, error: runtimeError } = useRuntimeConfig();

  useEffect(() => {
    let active = true;
    void loadFacebookSdk()
      .then((loaded) => {
        if (!active || !bootstrap.app_id || !bootstrap.graph_version) return;
        loaded.init({
          appId: bootstrap.app_id,
          version: bootstrap.graph_version,
          cookie: false,
          xfbml: false,
        });
        setSdk(loaded);
      })
      .catch((error: unknown) => {
        if (active)
          setError(
            error instanceof Error
              ? error.message
              : "Unable to load Meta sign-in",
          );
      });
    return () => {
      active = false;
      cleanupRef.current?.();
    };
  }, [bootstrap.app_id, bootstrap.graph_version, sdkAttempt]);

  function connect() {
    if (
      !sdk ||
      !bootstrap.embedded_signup_config_id ||
      !runtimeConfig ||
      busyRef.current
    )
      return;
    const extras = bootstrap.signup_extras[feature];
    if (!bootstrap.signup_version || !extras) {
      setError("Meta signup configuration is unavailable. Refresh and retry.");
      return;
    }
    busyRef.current = true;
    setError(null);
    setStage("popup");
    setPopupHint(false);
    let code: string | undefined;
    let assets:
      | { phone_number_id?: string; waba_id?: string; business_id?: string }
      | undefined;
    let active = true;
    let completing = false;
    const controller = new AbortController();
    const hintTimer = window.setTimeout(() => setPopupHint(true), 10_000);
    const timeout = window.setTimeout(
      () =>
        fail("Meta sign-in timed out. Allow popups for this site and retry."),
      10 * 60_000,
    );
    const cleanup = () => {
      active = false;
      busyRef.current = false;
      code = undefined;
      assets = undefined;
      window.clearTimeout(timeout);
      window.clearTimeout(hintTimer);
      window.removeEventListener("message", listener);
      controller.abort();
    };
    const fail = (message: string) => {
      if (!active) return;
      cleanup();
      setStage(null);
      setError(message);
    };
    const finish = () => {
      if (!active || completing || !code || !assets?.waba_id) return;
      completing = true;
      window.clearTimeout(hintTimer);
      setPopupHint(false);
      setStage("exchanging");
      void completeManagedOnboarding(
        "whatsapp",
        {
          code,
          ...assets,
          waba_id: assets.waba_id,
          label: label.trim(),
          ...(orgId ? { target_org_id: orgId } : {}),
        },
        setStage,
        controller.signal,
        runtimeConfig.api_base_url,
      )
        .then(async (result) => {
          if (!active) return;
          cleanup();
          setStage(null);
          await client.invalidateQueries({ queryKey: ["channel-bots"] });
          onConnected(result);
        })
        .catch((error: unknown) =>
          fail(
            error instanceof Error
              ? error.message
              : "Unable to connect WhatsApp",
          ),
        );
      code = undefined;
    };
    const listener = (event: MessageEvent) => {
      if (!active || completing) return;
      const message = parseEmbeddedSignupEvent(event);
      if (!message) return;
      if (
        message.event === "ERROR" ||
        message.data.error_code ||
        message.data.error_message
      ) {
        fail(
          "Meta could not complete signup. Retry or check your business account in Meta.",
        );
        return;
      }
      if (message.event === "CANCEL") {
        fail(
          message.data.current_step
            ? `Meta signup cancelled at ${message.data.current_step}.`
            : "Meta signup cancelled.",
        );
        return;
      }
      if (
        message.event === "FINISH_OBO_MIGRATION" ||
        message.event === "FINISH_GRANT_ONLY_API_ACCESS"
      ) {
        fail(
          "Select the Cloud API or WhatsApp Business app onboarding flow in Meta.",
        );
        return;
      }
      if ((message.data.waba_ids?.length ?? 0) > 1) {
        fail(
          "Select one WhatsApp Business Account for this bot and retry signup.",
        );
        return;
      }
      const waba = message.data.waba_id ?? message.data.waba_ids?.[0];
      if (!waba) {
        fail("Meta did not return a WhatsApp Business Account. Retry signup.");
        return;
      }
      assets = {
        phone_number_id: message.data.phone_number_id,
        waba_id: waba,
        business_id: message.data.business_id,
      };
      finish();
    };
    window.addEventListener("message", listener);
    cleanupRef.current = cleanup;
    try {
      sdk.login(
        (response) => {
          if (!active || completing) return;
          if (!response.authResponse?.code) {
            fail(
              "Meta sign-in was cancelled or blocked. Allow popups and retry.",
            );
            return;
          }
          code = response.authResponse.code;
          finish();
        },
        {
          config_id: bootstrap.embedded_signup_config_id,
          response_type: "code",
          override_default_response_type: true,
          extras,
        },
      );
    } catch {
      fail("Meta sign-in could not open. Allow popups and retry.");
    }
  }

  return (
    <div className="space-y-4">
      {error && <ErrorBanner message={error} />}
      {runtimeError && (
        <ErrorBanner message="Unable to load API configuration. Refresh and retry." />
      )}
      {error && !sdk && (
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            setError(null);
            setSdkAttempt((attempt) => attempt + 1);
          }}
        >
          <RefreshCw className="size-3" />
          Retry Meta sign-in
        </Button>
      )}
      <fieldset className="space-y-3" disabled={stage !== null}>
        <legend className="mb-3 text-xs font-medium">WhatsApp number</legend>
        {[
          { value: "", label: "New or existing Cloud API number" },
          {
            value: "whatsapp_business_app_onboarding",
            label: "I already use the WhatsApp Business app on this number",
          },
        ]
          .filter((choice) => bootstrap.feature_types.includes(choice.value))
          .map((choice) => (
            <label
              key={choice.value}
              className="flex items-start gap-2 text-xs"
            >
              <input
                type="radio"
                name="whatsapp-entry"
                value={choice.value}
                checked={feature === choice.value}
                onChange={() => setFeature(choice.value)}
                className="mt-0.5 accent-primary"
              />
              <span>{choice.label}</span>
            </label>
          ))}
      </fieldset>
      {stage && (
        <div
          role="status"
          className="flex items-center gap-2 text-xs text-muted-foreground"
        >
          <Loader2 className="size-3 animate-spin" />
          {stageLabels[stage] ?? "Waiting for Meta..."}
        </div>
      )}
      {popupHint && stage === "popup" && (
        <p className="text-xs text-muted-foreground">
          Complete signup in the Meta window. If no window opened, allow popups
          for this site.
        </p>
      )}
      <Button
        type="button"
        variant="primary"
        disabled={
          !sdk ||
          !runtimeConfig ||
          !label.trim() ||
          label.trim().length > 128 ||
          stage !== null
        }
        onClick={connect}
      >
        <ExternalLink className="size-3" />
        {!sdk && !error ? "Loading Meta..." : "Connect with Meta"}
      </Button>
      {stage === "popup" && (
        <Button
          type="button"
          variant="ghost"
          onClick={() => {
            cleanupRef.current?.();
            setStage(null);
          }}
        >
          Cancel
        </Button>
      )}
    </div>
  );
}
