import { useState } from "react";
import { toast } from "sonner";
import type { KeyInfo, CatalogEntry } from "@/types/keys";
import { useUpdateKey } from "@/hooks/use-keys";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { lanePriceLabel } from "@/schemas/platform-keys";
import { AddKeyDialog } from "./add-key-dialog";

export function CredentialBindingCard({
  service,
  catalog,
  readOnly,
}: {
  readonly service: KeyInfo;
  readonly catalog?: CatalogEntry;
  readonly readOnly: boolean;
}) {
  const [credential, setCredential] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [reconnect, setReconnect] = useState<KeyInfo | null>(null);
  const update = useUpdateKey();
  const platform =
    service.credential_binding === "platform" ||
    (!service.credential_binding &&
      service.auto_connected &&
      !service.api_key_id);
  const oauth =
    catalog?.provider_type === "oauth2" ||
    catalog?.provider_type === "device_code";
  const needsApp =
    oauth &&
    (catalog?.credential_mode === "user" ||
      (catalog?.credential_mode === "both" &&
        !catalog?.has_platform_oauth_credentials));
  async function switchBinding() {
    try {
      const result = await update.mutateAsync({
        keyId: service.id,
        use_platform_key: !platform,
        ...(platform && credential.trim()
          ? { credential: credential.trim() }
          : {}),
        ...(platform && needsApp
          ? {
              oauth_client_id: clientId.trim(),
              oauth_client_secret: clientSecret.trim(),
            }
          : {}),
      });
      setCredential("");
      setClientId("");
      setClientSecret("");
      if (platform && oauth && result.status === "pending_auth")
        setReconnect(result);
      toast.success("Credential binding updated");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not switch key",
      );
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-[15px]">Service key</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-xs">
        <p>
          {platform ? "Using NyxID's key" : "Using your own key"} —{" "}
          {!service.platform_key_pricing &&
          !service.byok_pricing &&
          catalog?.billing?.platform_billable
            ? "Current service/plan pricing applies"
            : lanePriceLabel(
                platform ? service.platform_key_pricing : service.byok_pricing,
              )}
        </p>
        {!readOnly && (
          <>
            {platform && !oauth && (
              <div className="space-y-2">
                <Label htmlFor="binding-credential">
                  Your replacement credential
                </Label>
                <Input
                  id="binding-credential"
                  type="password"
                  autoComplete="new-password"
                  value={credential}
                  onChange={(event) => setCredential(event.target.value)}
                />
              </div>
            )}
            {platform && needsApp && (
              <div className="space-y-2">
                <Label htmlFor="binding-client-id">
                  Your OAuth app client ID
                </Label>
                <Input
                  id="binding-client-id"
                  value={clientId}
                  onChange={(event) => setClientId(event.target.value)}
                />
                <Label htmlFor="binding-client-secret">
                  Your OAuth app client secret
                </Label>
                <Input
                  id="binding-client-secret"
                  type="password"
                  autoComplete="new-password"
                  value={clientSecret}
                  onChange={(event) => setClientSecret(event.target.value)}
                />
              </div>
            )}
            <p className="text-muted-foreground">
              Switching to NyxID's key keeps your saved personal credential for
              future use.
            </p>
            <Button
              disabled={
                update.isPending ||
                (platform
                  ? (!oauth && !credential.trim()) ||
                    (needsApp && (!clientId.trim() || !clientSecret.trim()))
                  : !service.platform_key_available || Boolean(service.node_id))
              }
              isLoading={update.isPending}
              onClick={() => void switchBinding()}
            >
              {platform ? "Use your own key" : "Use NyxID's key"}
            </Button>
            {platform && !service.auto_connected && (
              <Button
                disabled={update.isPending}
                onClick={() =>
                  update.mutate(
                    { keyId: service.id, is_active: !service.is_active },
                    { onError: (error) => toast.error(error.message) },
                  )
                }
              >
                {service.is_active ? "Disable" : "Enable"}
              </Button>
            )}
            {service.node_id && (
              <p className="text-muted-foreground">
                Remove node routing before choosing NyxID's key.
              </p>
            )}
          </>
        )}
        {reconnect && (
          <AddKeyDialog
            open
            onOpenChange={(open) => {
              if (!open) setReconnect(null);
            }}
            reconnectKey={reconnect}
            prefillSlug={service.catalog_service_slug ?? undefined}
          />
        )}
      </CardContent>
    </Card>
  );
}
