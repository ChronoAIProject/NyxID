import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { usePersonalLinkPreview } from "@/hooks/use-platform-personal";
import {
  personalLinkApi,
  telegramConfirmationUrl,
} from "@/lib/platform-personal";
import { CopyableField } from "@/components/shared/copyable-field";
import {
  readPersonalLink,
  clearPersonalLink,
  shortenPersonalLink,
  savePersonalChallenge,
  forgetPersonalChallenge,
} from "@/lib/platform-personal-handoff";
import {
  ACCESS_MODE_LABEL,
  ACCESS_MODE_DESCRIPTION,
} from "@/lib/assistant/nyxbot-labels";
import { type PersonalLinkReview } from "@/schemas/platform-personal";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { usePublicConfig } from "@/hooks/use-public-config";

export function PersonalLinkPage() {
  const actor = useAuthStore((s) => s.user?.id);
  const loading = useAuthStore((s) => s.isLoading);
  return (
    <PersonalLinkFlow
      key={actor ?? "anonymous"}
      actor={actor}
      authLoading={loading}
    />
  );
}
function PersonalLinkFlow({
  actor,
  authLoading,
}: {
  actor?: string;
  authLoading: boolean;
}) {
  const preview = usePersonalLinkPreview();
  const config = usePublicConfig();
  const client = useQueryClient();
  const [handoff] = useState(() => {
    const current = readPersonalLink();
    return current
      ? { expiresAt: current.expiresAt, challenge: current.challenge }
      : undefined;
  });
  const [challenge, setChallenge] = useState(() =>
    handoff?.challenge && handoff.challenge.accountId === actor
      ? handoff.challenge
      : undefined,
  );
  const [review, setReview] = useState<PersonalLinkReview>();
  const [command, setCommand] = useState<string>();
  const [replace, setReplace] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [denied, setDenied] = useState(false);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (preview.data) shortenPersonalLink(preview.data.expires_at);
  }, [preview.data]);
  const status = useQuery({
    queryKey: ["personal-link-status", actor, challenge?.id],
    enabled: !!actor && !!challenge && !review && !denied,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
    queryFn: () => personalLinkApi.status(challenge!.id, actor!),
    refetchInterval: (query) =>
      query.state.dataUpdateCount + query.state.errorUpdateCount < 120 &&
      challenge &&
      challenge.expiresAt > Date.now() &&
      ["review", "awaiting_chat_confirmation"].includes(
        query.state.data?.status ?? "",
      )
        ? 3000
        : false,
  });
  const terminal =
    status.data &&
    ["confirmed", "denied", "cancelled"].includes(status.data.status);
  useEffect(() => {
    if (terminal || denied) {
      clearPersonalLink();
      setCommand(undefined);
      void client.invalidateQueries({
        queryKey: ["platform-personal-bindings", actor],
      });
    }
  }, [terminal, denied, actor, client]);
  const intentExpired =
    !terminal &&
    !denied &&
    (!handoff ||
      Math.min(
        handoff.expiresAt,
        preview.data ? Date.parse(preview.data.expires_at) : Infinity,
      ) <= now);
  const challengeExpired =
    !!challenge &&
    (challenge.expiresAt <= now || status.data?.status === "expired");
  useEffect(() => {
    if (intentExpired || challengeExpired) {
      setCommand(undefined);
      setReview(undefined);
      if (intentExpired) clearPersonalLink();
      else forgetPersonalChallenge();
    }
  }, [intentExpired, challengeExpired]);
  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Could not complete this request.",
      );
    } finally {
      setBusy(false);
    }
  }
  function beginReview() {
    void run(async () => {
      const current = readPersonalLink();
      if (!current || !actor)
        throw new Error(
          "Reopen the original link in this browser after signing in.",
        );
      forgetPersonalChallenge();
      setChallenge(undefined);
      setReview(undefined);
      setCommand(undefined);
      setReplace(false);
      const result = await personalLinkApi.review(current.token, actor);
      savePersonalChallenge(result.challenge_id, actor, result.expires_at);
      setChallenge({
        id: result.challenge_id,
        accountId: actor,
        expiresAt: Date.parse(result.expires_at),
      });
      setReview(result);
    });
  }
  const chat = review?.chat ?? status.data?.chat ?? preview.data?.chat;
  const chatName = chat?.display_name ?? "chat";
  const telegramReturn =
    chat?.platform === "telegram"
      ? telegramConfirmationUrl(command, preview.data?.manager.username)
      : undefined;
  const loginInstruction =
    chat?.login_command ??
    "the login control shown in the original private chat";
  const returnTo =
    typeof window !== "undefined"
      ? `${window.location.origin}/nyxbot/link`
      : "";
  return (
    <main className="mx-auto max-w-lg space-y-5 px-4 py-10" data-sensitive>
      <meta name="referrer" content="no-referrer" />
      <PageHeader
        title={`Use your NyxBot in ${chatName}`}
        description="Link only if you requested this in the bot's private chat."
      />
      {status.data?.status === "confirmed" ? (
        <section className="space-y-3 rounded-xl border border-border bg-card p-5">
          <p role="status">{chatName} is linked.</p>
          <p className="text-12 text-muted-foreground">
            {chat?.question_instruction} Your dedicated {chatName} thread and
            approvals will be available in NyxID.
          </p>
          <Button asChild variant="primary">
            <a href="/assistant?panel=nyxbot">Open NyxBot</a>
          </Button>
        </section>
      ) : denied ||
        status.data?.status === "denied" ||
        status.data?.status === "cancelled" ? (
        <p role="status">
          Linking was cancelled. No new personal access was granted. Send{" "}
          {loginInstruction} to start again.
        </p>
      ) : intentExpired ? (
        <ErrorBanner
          message={`This linking request is missing or expired. Reopen the original link in this browser, or use ${loginInstruction} again in the bot's private chat. Storage must be available to continue after navigating away.`}
        />
      ) : (
        <>
          {preview.isPending && (
            <p role="status">Checking your linking request...</p>
          )}
          {preview.error && (
            <ErrorBanner
              message={preview.error.message}
              onRetry={() => void preview.refetch()}
            />
          )}
          {preview.data && (
            <section className="space-y-3 rounded-xl border border-border bg-card p-5 text-12">
              <p>
                Bot: <strong>{preview.data.manager.display_name}</strong>
              </p>
              <p>
                {chatName} account:{" "}
                {preview.data.sender.display_name
                  ? `${preview.data.sender.display_name} (${preview.data.sender.id})`
                  : preview.data.sender.id}
              </p>
              {!actor ? (
                <>
                  <p>
                    Sign in to review personal access. Registration and browser
                    sign-in do not link this chat automatically.
                  </p>
                  {(config.data?.invite_code_required ?? true) && (
                    <p>
                      An invitation is required to register. Use your existing
                      invitation, or follow the invitation instructions on the
                      registration page. Existing users can sign in directly.
                    </p>
                  )}
                  {config.error && (
                    <p role="alert">
                      Registration settings are unavailable. Invitation
                      requirements remain enforced.
                    </p>
                  )}
                  <div className="flex flex-wrap gap-2">
                    <Button asChild variant="primary" disabled={authLoading}>
                      <a
                        href={`/login?return_to=${encodeURIComponent(returnTo)}`}
                      >
                        Sign in
                      </a>
                    </Button>
                    <Button asChild variant="outline" disabled={authLoading}>
                      <a
                        href={`/register?return_to=${encodeURIComponent(returnTo)}`}
                      >
                        Register
                      </a>
                    </Button>
                  </div>
                  <p className="text-muted-foreground">
                    After registering, sign in to continue this account review.
                    If you use another tab, return here after signing in. This
                    request lasts up to 24 hours.
                  </p>
                </>
              ) : (!review || challengeExpired) &&
                (!challenge ||
                  challengeExpired ||
                  status.data?.status === "expired" ||
                  status.data?.status === "review") ? (
                <Button
                  variant="primary"
                  isLoading={busy}
                  onClick={beginReview}
                >
                  {challenge ? "Review again" : "Review personal access"}
                </Button>
              ) : null}
            </section>
          )}
          {review && !challengeExpired && (
            <section className="space-y-4 rounded-xl border border-border bg-card p-5 text-12">
              <h2 className="text-15 font-semibold">Review personal access</h2>
              <p>
                NyxID account:{" "}
                {review.account.display_name ?? review.account.id}
              </p>
              <p>
                Permission mode:{" "}
                <strong>{ACCESS_MODE_LABEL[review.permission_mode]}</strong>.{" "}
                {ACCESS_MODE_DESCRIPTION[review.permission_mode]}
              </p>
              <p>
                This {chatName} chat uses your existing NyxBot access to
                connected services and account management, including existing
                memory and persona. Changes to your NyxBot settings apply here
                too.
              </p>
              <p>{chat?.question_instruction}</p>
              <p>
                Send {chat?.status_command} to check this chat's live login.
                Send {chat?.logout_command} to disconnect it or cancel pending
                confirmations. Browser sign-out does not disconnect this chat.
              </p>
              <p>
                Personal work uses your normal billing and credits. The public
                sponsor does not pay for your personal turns. Existing
                organization resource billing rules still apply.
              </p>
              <p>
                Chat confirmations can approve sensitive actions, including
                destructive actions, machine access and saved logins. Your
                current permission mode determines when NyxBot asks for
                confirmation.
              </p>
              <p>{chat?.transport_disclosure}</p>
              <p>
                This creates a new personal thread. Public chat and your old
                home transcript are not imported. Work already started may keep
                running in NyxID after unlink; stop it from the app. Saved
                history and pending cards remain available there.
              </p>
              {review.replaces_existing_binding && (
                <label className="flex items-start gap-2">
                  <Checkbox
                    checked={replace}
                    onCheckedChange={(v) => setReplace(v === true)}
                  />
                  <span>
                    I agree to replace the existing personal link for this
                    {chatName} account. Its previous history will not transfer.
                  </span>
                </label>
              )}
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="primary"
                  isLoading={busy}
                  disabled={review.replaces_existing_binding && !replace}
                  onClick={() =>
                    void run(async () => {
                      const result = await personalLinkApi.approve(
                        review.challenge_id,
                        review.review_digest,
                        replace,
                        actor!,
                      );
                      setCommand(result.confirmation_command ?? undefined);
                      setReview(undefined);
                      void client.invalidateQueries({
                        queryKey: [
                          "personal-link-status",
                          actor,
                          review.challenge_id,
                        ],
                      });
                    })
                  }
                >
                  Approve personal access
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await personalLinkApi.deny(review.challenge_id, actor!);
                      setDenied(true);
                      setReview(undefined);
                    })
                  }
                >
                  Deny
                </Button>
              </div>
              <p className="text-muted-foreground">
                Approve here, then confirm in the same private {chatName} chat
                within 10 minutes.
              </p>
            </section>
          )}
          {!review && challenge && !challengeExpired && (
            <section className="space-y-3 rounded-xl border border-border bg-card p-5 text-12">
              <p role="status">
                Waiting for confirmation from the same private {chatName} chat.
                Browser approval alone does not grant access.
              </p>
              {command ? (
                <>
                  {telegramReturn && (
                    <Button variant="primary" asChild>
                      <a
                        href={telegramReturn}
                        target="_blank"
                        rel="noopener noreferrer"
                      >
                        Open Telegram to finish
                      </a>
                    </Button>
                  )}
                  <p>
                    {chat?.platform === "discord"
                      ? "Choose the /nyx confirm application command in the same bot DM and enter the shown P-… value in its code option. Ordinary text does not reach this integration."
                      : "Send this one-time command in the private chat:"}
                  </p>
                  <CopyableField label="Confirmation command" value={command} />
                </>
              ) : (
                <p>
                  The one-time command is shown only after approval. If you lost
                  it, review again to request a new one.
                </p>
              )}
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  onClick={() => void status.refetch()}
                  disabled={status.isFetching}
                >
                  Refresh status
                </Button>
                <Button variant="outline" onClick={beginReview} disabled={busy}>
                  Review again
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await personalLinkApi.deny(challenge.id, actor!);
                      setDenied(true);
                    })
                  }
                >
                  Cancel linking
                </Button>
              </div>
            </section>
          )}
          {challengeExpired && actor && (
            <p role="status" className="text-12">
              The 10-minute confirmation expired. Review again while the
              original linking request is valid.
            </p>
          )}
          {(error || status.error) && (
            <>
              <ErrorBanner message={error ?? status.error!.message} />
              {actor && (
                <Button variant="outline" onClick={beginReview} disabled={busy}>
                  Start a fresh review
                </Button>
              )}
            </>
          )}
        </>
      )}
      <p className="text-12 text-muted-foreground">
        Looking for child-bot setup?{" "}
        <a className="underline" href="/onboarding">
          Open BotFather setup
        </a>
        .
      </p>
    </main>
  );
}
