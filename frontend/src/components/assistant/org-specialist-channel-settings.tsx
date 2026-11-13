import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
import { useOrgSpecialistChannel } from "@/hooks/use-org-specialist-channel";


export function OrgSpecialistChannelSettings({ id }: { readonly id: string }) {
  useUiTranslation();
  const [challenge, setChallenge] = useState<string>();
  const [expiresAt, setExpiresAt] = useState<string>();
  const [expired, setExpired] = useState(false);
  const api = useOrgSpecialistChannel(id, expired ? undefined : challenge);
  useEffect(() => {
    if (!expiresAt) return;
    const remaining = Date.parse(expiresAt) - Date.now();
    const timer = setTimeout(() => setExpired(true), Math.max(0, remaining));
    return () => clearTimeout(timer);
  }, [expiresAt]);
  const error =
    api.settings.error ??
    api.issue.error ??
    api.confirm.error ??
    api.revoke.error ??
    (expired ? null : api.status.error);
  return (
    <section data-member-verification="" tabIndex={-1} aria-label={translateUi("Verify my chat account")} className="space-y-3 scroll-mt-4 border-t border-border pt-3 type-secondary focus-visible:outline focus-visible:outline-ring">
      <h3 className="type-card-title">{translateUi("Verify my chat account")}</h3>
      <p className="text-muted-foreground">{translateUi("Each member, including admins, verifies individually in their own NyxID session. Generate a code, send it privately to this bot, wait for NyxID to observe it, then confirm your account here. App credentials and the platform callback challenge do not verify a member. Group replies are public; your private conversation stays private.")}</p>
      {error ? (
        <p role="alert" className="text-destructive">
          {error.message}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={api.issue.isPending}
          onClick={() => {
            api.confirm.reset();
            api.revoke.reset();
            void api.issue
              .mutateAsync()
              .then((value) => {
                setChallenge(value.id);
                setExpiresAt(value.expires_at);
                setExpired(Boolean(value.expires_at && Date.parse(value.expires_at) <= Date.now()));
              })
              .catch(() => {});
          }}
        >
          {expired ? translateUi("Generate a new code") : translateUi("Verify my chat account")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={api.revoke.isPending}
          onClick={() => api.revoke.mutate(undefined, { onSuccess: () => { setChallenge(undefined); setExpiresAt(undefined); setExpired(false); api.confirm.reset(); } })}
        >{translateUi("Unlink my chat accounts")}</Button>
      </div>
      {expired && !api.confirm.isSuccess ? <p role="status">{translateUi("This verification code expired. Generate a new code and send it privately to the bot, then confirm here.")}</p> : null}
      {api.issue.data?.code && challenge && !expired && !api.confirm.isSuccess ? (
        <div className="space-y-2">
          <p>{translateUi("Send this code in a private message (DM) to the bot, then return here. Never send it to a group. This code expires in 10 minutes.")}</p>
          <CopyableField
            value={api.issue.data.code}
            label={translateUi("Member verification code")}
          />
          {api.status.data?.state === "observed" ? (
            <>
              <p>{translateUi("Confirm that “")}{api.status.data.sender_label ?? translateUi("Chat account")}{translateUi("” is your account. Its messages use your member permissions and billing.")}</p>
              <Button
                size="sm"
                variant="primary"
                disabled={api.confirm.isPending || Boolean(api.status.error)}
                onClick={() => api.confirm.mutate(undefined, { onSuccess: () => { setChallenge(undefined); setExpiresAt(undefined); setExpired(false); } })}
              >{translateUi("Confirm my account")}</Button>
            </>
          ) : (
            <p className="text-text-tertiary">{translateUi("Waiting for NyxID to observe your private message. Confirmation becomes available after the bot receives the code.")}</p>
          )}
        </div>
      ) : null}
      {!api.settings.error && !api.revoke.isSuccess && (api.confirm.isSuccess || api.settings.data?.member_verified) ? (
        <p role="status">{translateUi("Your chat account is verified.")}</p>
      ) : null}
      {api.revoke.isSuccess ? (
        <p role="status">{translateUi("Your chat accounts are unlinked.")}</p>
      ) : null}
    </section>
  );
}
