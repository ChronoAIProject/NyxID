import { useMemo, useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Shield, Pencil } from "lucide-react";
import { toast } from "sonner";

import type { CredentialSource } from "@/schemas/orgs";

function sameOwner(
  a?: CredentialSource,
  b?: CredentialSource,
): boolean {
  const aType = a?.type ?? "personal";
  const bType = b?.type ?? "personal";
  if (aType !== bType) return false;
  if (aType === "personal") return true;
  return a?.type === "org" && b?.type === "org" && a.org_id === b.org_id;
}

export function ServiceScopeCard({
  keyId,
  allowAllServices,
  allowedServiceIds,
  allowedServices,
  apiKeySource,
}: {
  readonly keyId: string;
  readonly allowAllServices: boolean;
  readonly allowedServiceIds: readonly string[];
  readonly allowedServices: readonly {
    readonly id: string;
    readonly slug: string;
    readonly label: string;
    readonly catalog_service_name: string | null;
  }[];
  readonly apiKeySource?: CredentialSource;
}) {
  const [editing, setEditing] = useState(false);
  const [allowAll, setAllowAll] = useState(allowAllServices);
  const [selectedIds, setSelectedIds] =
    useState<readonly string[]>(allowedServiceIds);
  const { data: allKeys } = useKeys();
  // Filter to services owned by the same owner as the API key. Personal
  // API keys can only scope to personal services; org API keys can only
  // scope to the same org's services. The backend
  // (`key_service::validate_service_ids`) enforces this server-side; the
  // filter here is to avoid offering options that would 400 on save.
  const personalKeys = useMemo(
    () =>
      (allKeys ?? []).filter(
        (k) =>
          !k.auto_connected &&
          k.is_active &&
          sameOwner(k.credential_source, apiKeySource),
      ),
    [allKeys, apiKeySource],
  );
  const updateApiKey = useUpdateApiKey();

  function handleSave() {
    updateApiKey.mutate(
      {
        keyId,
        allow_all_services: allowAll,
        allowed_service_ids: allowAll ? [] : [...selectedIds],
      },
      {
        onSuccess: () => {
          toast.success("Service scope updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update scope";
          toast.error(message);
        },
      },
    );
  }

  function handleCancel() {
    setAllowAll(allowAllServices);
    setSelectedIds(allowedServiceIds);
    setEditing(false);
  }

  function toggleService(serviceId: string) {
    setSelectedIds((prev) =>
      prev.includes(serviceId)
        ? prev.filter((id) => id !== serviceId)
        : [...prev, serviceId],
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Shield className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Service Scope</CardTitle>
          </div>
          {!editing && (
            <Button
              size="icon"
              variant="ghost"
              onClick={() => setEditing(true)}
            >
              <Pencil className="h-4 w-4" />
            </Button>
          )}
        </div>
        <CardDescription>
          Which external services this key can access via proxy
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {editing ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Checkbox
                id="allow-all-services"
                checked={allowAll}
                onCheckedChange={(checked) => setAllowAll(checked === true)}
              />
              <Label htmlFor="allow-all-services" className="text-[12px]">
                Allow all services
              </Label>
            </div>

            {!allowAll && (
              <div className="space-y-2 rounded-lg border border-border p-3">
                <p className="text-xs text-muted-foreground">
                  Select allowed services:
                </p>
                {personalKeys.length > 0 ? (
                  personalKeys.map((k) => (
                    <div key={k.id} className="flex items-center gap-2">
                      <Checkbox
                        id={`svc-${k.id}`}
                        checked={selectedIds.includes(k.id)}
                        onCheckedChange={() => toggleService(k.id)}
                      />
                      <Label htmlFor={`svc-${k.id}`} className="text-xs">
                        {k.label}
                        <span className="text-muted-foreground">
                          {" "}
                          ({k.slug})
                        </span>
                      </Label>
                    </div>
                  ))
                ) : (
                  <p className="text-xs text-muted-foreground">
                    No external services configured yet.
                  </p>
                )}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <Button
                variant="primary"
                onClick={handleSave}
                disabled={updateApiKey.isPending}
              >
                Save
              </Button>
              <Button variant="outline" onClick={handleCancel}>
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {allowAllServices ? (
              <Badge variant="secondary">All services</Badge>
            ) : allowedServices.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {allowedServices.map((s) => (
                  <Badge key={s.id} variant="secondary" className="text-xs">
                    {s.label} ({s.slug})
                  </Badge>
                ))}
              </div>
            ) : (
              <Badge variant="destructive">No services (auth-only)</Badge>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
