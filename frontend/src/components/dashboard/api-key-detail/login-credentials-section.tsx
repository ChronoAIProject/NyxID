import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useEffect, useState } from "react";
import { Ban } from "lucide-react";
import {
  useLoginCredentials,
  useRevokeLoginCredential,
} from "@/hooks/use-agent-key-login";
import type { LoginCredential } from "@/schemas/agent-key-login";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";

const timestamp = (value: string | null) =>
  value ? new Date(value).toLocaleString(getFormattingLocale()) : "Never";

export function LoginCredentialsSection({
  keyId,
  canWrite,
}: {
  keyId: string;
  canWrite: boolean;
}) {
  useUiTranslation();
  const credentials = useLoginCredentials(keyId);
  const revoke = useRevokeLoginCredential(keyId);
  const [selected, setSelected] = useState<LoginCredential | null>(null);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return (
    <section className="space-y-4 border-t border-border/50 pt-6">
      <div>
        <h2 className="type-card-title">{translateUi("Login credentials")}</h2>
        <p className="mt-1 text-14 text-muted-foreground">{translateUi("Revoking or rotating this key also invalidates all its login credentials.")}</p>
      </div>
      {credentials.isLoading && (
        <p role="status" className="text-14 text-muted-foreground">{translateUi("Loading credentials...")}</p>
      )}
      {credentials.error && (
        <ErrorBanner
          message={translateUi("Could not load login credentials.")}
          onRetry={credentials.refetch}
        />
      )}
      {credentials.data?.length === 0 && (
        <p className="text-14 text-muted-foreground">{translateUi("No login credentials.")}</p>
      )}
      <div className="divide-y divide-border/50">
        {credentials.data?.map((credential) => {
          const expired =
            credential.expires_at !== null &&
            new Date(credential.expires_at).getTime() <= now;
          return (
            <article
              key={credential.id}
              className="flex flex-wrap items-start justify-between gap-3 py-3 text-14"
            >
              <div className="min-w-0 space-y-2">
                <div className="flex flex-wrap items-center gap-2">
                  <strong className="break-words">{credential.label}</strong>
                  <Badge
                    variant={
                      credential.is_active && !expired ? "success" : "secondary"
                    }
                  >
                    {!credential.is_active
                      ? translateUi("Revoked")
                      : expired
                        ? translateUi("Expired")
                        : translateUi("Active")}
                  </Badge>
                </div>
                <p className="font-mono">{credential.secret_prefix}</p>
                <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 [&_dt]:text-muted-foreground [&_dd]:break-words">
                  <dt>{translateUi("Issued")}</dt>
                  <dd>{timestamp(credential.created_at)}</dd>
                  <dt>{translateUi("Last used")}</dt>
                  <dd>{timestamp(credential.last_used_at)}</dd>
                  <dt>{translateUi("Expires")}</dt>
                  <dd>
                    {credential.expires_at
                      ? timestamp(credential.expires_at)
                      : translateUi("No expiry")}
                  </dd>
                  {credential.revoked_reason && (
                    <>
                      <dt>{translateUi("Revoked reason")}</dt>
                      <dd>{credential.revoked_reason.replaceAll("_", " ")}</dd>
                    </>
                  )}
                </dl>
              </div>
              {canWrite && credential.is_active && (
                <Button
                  variant="destructive"
                  onClick={() => {
                    revoke.reset();
                    setSelected(credential);
                  }}
                >
                  <Ban className="size-3" />{translateUi("Revoke")}</Button>
              )}
            </article>
          );
        })}
      </div>
      <Dialog
        open={canWrite && selected !== null}
        onOpenChange={(open) => {
          if (!open && !revoke.isPending) setSelected(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Revoke login credential")}</DialogTitle>
            <DialogDescription>{translateUi("Revoke the login credential for ")}{selected?.label}{translateUi("? This CLI will need to authorize again.")}</DialogDescription>
          </DialogHeader>
          {revoke.error && (
            <ErrorBanner message={translateUi("Could not revoke this login credential. Try again.")} />
          )}
          <DialogFooter>
            <Button
              disabled={revoke.isPending}
              onClick={() => setSelected(null)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={revoke.isPending}
              disabled={revoke.isPending}
              onClick={() => {
                if (canWrite && selected)
                  void revoke
                    .mutateAsync(selected.id)
                    .then(() => setSelected(null))
                    .catch(() => {});
              }}
            >
              <Ban className="size-3" />{translateUi("Revoke")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
