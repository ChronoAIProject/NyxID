import { useEffect, useRef, useState } from "react";
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
import type { ManagedBootstrap } from "@/schemas/channel-managed";
import type { CreateChannelBotResponse } from "@/types/channels";

const stageLabels: Record<string, string> = {
  exchanging: "Exchanging...",
  subscribing: "Subscribing...",
  registering: "Registering...",
};

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
    if (!sdk || !bootstrap.embedded_signup_config_id || busyRef.current) return;
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
      if (message.event === "CANCEL") {
        fail(
          message.data.current_step
            ? `Meta signup cancelled at ${message.data.current_step}.`
            : "Meta signup cancelled.",
        );
        return;
      }
      if (message.event === "ERROR") {
        fail(
          "Meta could not complete signup. Retry or check your business account in Meta.",
        );
        return;
      }
      if (!message.data.waba_id) {
        fail("Meta did not return a WhatsApp Business Account. Retry signup.");
        return;
      }
      assets = message.data;
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
          extras: { setup: {}, featureType: feature, sessionInfoVersion: "3" },
        },
      );
    } catch {
      fail("Meta sign-in could not open. Allow popups and retry.");
    }
  }

  return (
    <div className="space-y-4">
      {error && <ErrorBanner message={error} />}
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
          !sdk || !label.trim() || label.trim().length > 128 || stage !== null
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
