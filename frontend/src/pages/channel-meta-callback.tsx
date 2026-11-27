import { metaProgressSchema, type MetaProgress } from "@/schemas/channel-meta";
import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/api-client";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useAuthStore } from "@/stores/auth-store";

export function ChannelMetaCallbackPage() {
  const { t } = useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  const checkingSession = useAuthStore((s) => s.isLoading);
  const exchange = useRef<{
    actor: string;
    result: Promise<MetaProgress>;
  } | null>(null);
  const [input] = useState(() => {
    const params = new URLSearchParams(window.location.search);
    return {
      state: params.get("state"),
      code: params.get("code"),
      platform: window.location.pathname.split("/").at(-1),
    };
  });
  const valid = Boolean(
    input.state && ["messenger", "instagram"].includes(input.platform ?? ""),
  );
  const [message, setMessage] = useState(() =>
    valid
      ? t("Completing inbox sign-in…")
      : t("This connection request is invalid. Return to setup and reconnect."),
  );
  useEffect(() => {
    // Keep the authorization code out of browser history and login redirects,
    // including when the NyxID session expired during provider consent.
    window.history.replaceState(
      window.history.state,
      "",
      window.location.pathname,
    );
    if (!actor || !valid) return;
    if (!exchange.current) {
      const result = api
        .post(`/channel-bots/meta/${input.platform}/exchange`, {
          state: input.state,
          code: input.code ?? null,
        })
        .then((result) => metaProgressSchema.parse(result));
      exchange.current = { actor, result };
    }
    const sameActor = exchange.current.actor === actor;
    let active = true;
    void exchange.current.result
      .then((result) => {
        if (!active) return;
        if (!sameActor) {
          setMessage(
            t(
              "Sign in to NyxID again, then return to inbox setup to reconnect.",
            ),
          );
        } else if (
          result.status === "assets" ||
          result.status === "completed"
        ) {
          setMessage(
            t(
              "Sign-in completed. Return to the setup window to select your inbox.",
            ),
          );
          window.close();
        } else if (result.status === "cancelled") {
          setMessage(
            t(
              "Sign-in was cancelled. No inbox was connected. Return to setup to reconnect.",
            ),
          );
        } else if (result.status === "revoked") {
          setMessage(
            t("Inbox consent was revoked. Return to setup and reconnect."),
          );
        } else if (
          result.status === "pending" ||
          result.status === "exchanging"
        ) {
          setMessage(
            t(
              "Sign-in is still processing. Return to setup to check its status.",
            ),
          );
        } else {
          setMessage(t("Sign-in failed. Return to setup and reconnect."));
        }
      })
      .catch((error: unknown) => {
        if (!active) return;
        setMessage(
          !sameActor
            ? t(
                "Sign in to NyxID again, then return to inbox setup to reconnect.",
              )
            : error instanceof Error
              ? error.message
              : t("Sign-in failed. Return to setup and reconnect."),
        );
      });
    return () => {
      active = false;
    };
  }, [actor, input, t, valid]);
  return (
    <main className="mx-auto max-w-lg p-6">
      <p role="status" className="type-body text-foreground">
        {!actor && !checkingSession
          ? t(
              "Sign in to NyxID again, then return to inbox setup to reconnect.",
            )
          : message}
      </p>
    </main>
  );
}
