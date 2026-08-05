import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  openOAuthChannel,
  postOAuthAck,
  postOAuthRetry,
} from "@/lib/oauth-popup";
import {
  isOAuthActionMessage,
  isOAuthResultMessage,
  validateAuthorizationUrl,
} from "@/schemas/oauth-popup";
import { useOAuthPopupStore } from "@/stores/oauth-popup-store";

export interface OAuthPopupRetryResult {
  readonly nextNonce: string;
  readonly url: string;
}

export function useOAuthPopupReceiver({
  launchId,
  onRetry,
  onViewResult,
  onDismiss,
}: {
  readonly launchId: string | null;
  readonly onRetry: () => Promise<OAuthPopupRetryResult>;
  readonly onViewResult: (keyId: string) => boolean;
  readonly onDismiss: () => void;
}): void {
  const queryClient = useQueryClient();
  const attempt = useOAuthPopupStore((state) => state.attempt);

  useEffect(() => {
    if (!launchId || attempt?.launchId !== launchId || !attempt.nonce) return;
    const currentNonce = attempt.nonce;
    const channel = openOAuthChannel(currentNonce);
    if (!channel) return;
    let live = true;
    let retrying = false;

    channel.onmessage = (event: MessageEvent<unknown>) => {
      if (!live) return;
      if (isOAuthResultMessage(event.data)) {
        void queryClient.invalidateQueries({ queryKey: ["keys"] });
        if (attempt.keyId) {
          void queryClient.invalidateQueries({
            queryKey: ["keys", attempt.keyId],
          });
        }
        return;
      }
      if (!isOAuthActionMessage(event.data)) return;

      if (event.data.action === "view_result") {
        if (attempt.keyId && onViewResult(attempt.keyId)) postOAuthAck(channel);
        return;
      }
      if (event.data.action === "dismiss") {
        onDismiss();
        postOAuthAck(channel);
        return;
      }
      if (retrying) return;
      retrying = true;

      void onRetry()
        .then((retry) => {
          const current = useOAuthPopupStore.getState().attempt;
          if (current?.launchId !== launchId || current.nonce !== currentNonce)
            return;
          const authorizationUrl = validateAuthorizationUrl(
            retry.url,
            retry.nextNonce,
          );
          if (authorizationUrl === null) return;
          postOAuthRetry(channel, {
            type: "oauth_retry",
            nextNonce: retry.nextNonce,
            url: authorizationUrl.href,
          });
          useOAuthPopupStore.getState().setNonce(launchId, retry.nextNonce);
        })
        .catch(() => {
          retrying = false;
          // The opener keeps the dialog's inline error/poll state authoritative.
        });
    };

    return () => {
      live = false;
      channel.close();
    };
  }, [attempt, launchId, onDismiss, onRetry, onViewResult, queryClient]);
}
