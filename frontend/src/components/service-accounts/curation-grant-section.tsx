import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useIssueCurationGrant,
  useRevokeCurationGrant,
} from "@/hooks/use-service-accounts";
import {
  curationGrantSchema,
  type CurationGrantFormData,
} from "@/schemas/service-accounts";
import type { ServiceAccount } from "@/types/service-accounts";
import { useAuthStore } from "@/stores/auth-store";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { toast } from "sonner";

export function CurationGrantSection({
  account,
}: {
  readonly account: ServiceAccount;
}) {
  const isAdmin = useAuthStore((state) => state.user?.is_admin ?? false);
  const issue = useIssueCurationGrant();
  const revoke = useRevokeCurationGrant();
  const [editing, setEditing] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const form = useAppForm<CurationGrantFormData>({
    resolver: zodResolver(curationGrantSchema),
    defaultValues: {
      service_ids: "",
      ornn_proxy_service_id: "",
      expires_at: "",
      max_writes: "100",
      window_seconds: "3600",
    },
  });
  const grant = account.curation_grant;
  const expired =
    grant?.expires_at != null && Date.parse(grant.expires_at) <= now;

  function openForm() {
    setSubmitError(null);
    form.reset({
      service_ids: grant?.service_ids.join(", ") ?? "",
      ornn_proxy_service_id: grant?.ornn_proxy_service_id ?? "",
      expires_at: grant?.expires_at ?? "",
      max_writes: String(grant?.max_writes ?? 100),
      window_seconds: String(grant?.window_seconds ?? 3600),
    });
    setEditing(true);
  }

  async function submit(values: CurationGrantFormData) {
    setSubmitError(null);
    try {
      await issue.mutateAsync({
        saId: account.id,
        data: {
          service_ids: values.service_ids.split(/[,\s]+/).filter(Boolean),
          ornn_proxy_service_id: values.ornn_proxy_service_id || undefined,
          expires_at: values.expires_at
            ? new Date(values.expires_at).toISOString()
            : undefined,
          max_writes: Number(values.max_writes),
          window_seconds: Number(values.window_seconds),
        },
      });
      setEditing(false);
      toast.success("Curation grant issued");
    } catch (error) {
      setSubmitError(
        error instanceof ApiError
          ? error.message
          : "Could not issue curation grant",
      );
    }
  }

  async function revokeGrant() {
    try {
      await revoke.mutateAsync(account.id);
      toast.success("Curation grant revoked");
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : "Could not revoke curation grant",
      );
    }
  }

  return (
    <DetailSection title="Catalog skill curation">
      <DetailRow
        label="Purpose"
        value={account.purpose === "curation" ? "Curation" : "General"}
      />
      <DetailRow
        label="Management"
        value={
          account.platform_protected
            ? "Platform admin only"
            : "Standard account access"
        }
      />
      <DetailRow
        label="Credential generation"
        value={String(account.credential_generation ?? 0)}
      />
      <DetailRow
        label="Grant"
        value={grant ? (expired ? "Expired" : "Active") : "None"}
      />
      {grant && (
        <>
          <DetailRow label="Grant ID" value={grant.id} copyable />
          <DetailRow
            label="Catalog service IDs"
            value={grant.service_ids.join(", ")}
          />
          <DetailRow
            label="Ornn proxy target"
            value={grant.ornn_proxy_service_id ?? "Disabled"}
          />
          <DetailRow
            label="Write budget"
            value={`${grant.writes_used} / ${grant.max_writes} per ${grant.window_seconds}s`}
          />
          <DetailRow
            label="Expires"
            value={
              grant.expires_at ? formatDate(grant.expires_at) : "No expiry"
            }
          />
        </>
      )}
      <div className="space-y-4 p-5">
        <p className="text-[12px] text-muted-foreground">
          Recommendations can be assigned, replaced, removed, and restored
          within these catalog services. Package editing requires a separate
          scoped Ornn identity. Revoking the grant keeps this account protected.
        </p>
        {isAdmin && !editing && (
          <div className="flex justify-end gap-2">
            {grant && (
              <Button
                variant="destructive"
                isLoading={revoke.isPending}
                onClick={() => void revokeGrant()}
              >
                Revoke grant
              </Button>
            )}
            <Button onClick={openForm}>
              {grant ? "Replace grant" : "Issue grant"}
            </Button>
          </div>
        )}
        {isAdmin && editing && (
          <Form {...form}>
            <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
              <p className="text-[12px] text-muted-foreground">
                Set account scopes to catalog:skills:read and
                catalog:skills:write. Add proxy only when an exact Ornn target
                is configured. Org-owned accounts cannot receive a grant.
              </p>
              {(
                [
                  [
                    "service_ids",
                    "Catalog service UUIDs",
                    "Comma-separated UUIDs",
                  ],
                  [
                    "ornn_proxy_service_id",
                    "Ornn catalog service UUID (optional)",
                    "Exact HTTP proxy target",
                  ],
                  ["expires_at", "Expiry (optional)", "2026-12-31T23:59:59Z"],
                  ["max_writes", "Maximum writes per window", "100"],
                  ["window_seconds", "Window in seconds", "3600"],
                ] as const
              ).map(([name, label, placeholder]) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{label}</FormLabel>
                      <FormControl>
                        <Input {...field} placeholder={placeholder} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
              {submitError && (
                <p role="alert" className="text-[12px] text-destructive">
                  {submitError}
                </p>
              )}
              <div className="flex justify-end gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setEditing(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  isLoading={issue.isPending}
                  disabled={!form.formState.isDirty || !form.formState.isValid}
                >
                  Issue grant
                </Button>
              </div>
            </form>
          </Form>
        )}
      </div>
    </DetailSection>
  );
}
