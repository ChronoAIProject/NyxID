import { useVerifyAppBranding } from "@/hooks/use-oauth-branding";
import type { AdminOAuthClient } from "@/types/admin";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";

export function BrandingVerification({
  client,
}: {
  readonly client: AdminOAuthClient;
}) {
  const update = useVerifyAppBranding(client.id);
  const verified =
    client.branding_verified_revision != null &&
    client.branding_verified_revision === client.branding_revision;
  return (
    <div className="space-y-2">
      {client.logo_url && (
        <img
          src={client.logo_url}
          alt={`${client.client_name} logo`}
          className="size-10 rounded object-contain"
        />
      )}
      {client.homepage_url && (
        <a
          href={client.homepage_url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs text-muted-foreground underline"
        >
          Homepage
        </a>
      )}
      {client.handoff_blurb && (
        <p className="text-xs text-muted-foreground">{client.handoff_blurb}</p>
      )}
      <label className="flex items-center gap-2 text-xs">
        <Switch
          aria-label={`Verify branding for ${client.client_name}`}
          checked={verified}
          disabled={update.isPending}
          onCheckedChange={(value) =>
            update.mutate({
              verified: value,
              branding_revision: client.branding_revision ?? 0,
            })
          }
        />
        Verified branding
      </label>
      {update.error && <ErrorBanner message={update.error.message} />}
    </div>
  );
}
