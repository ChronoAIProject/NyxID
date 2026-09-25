import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useSaKeyReadGrant, useSaveSaKeyReadGrant, useRevokeSaKeyReadGrant } from "@/hooks/use-sa-key-read-grant";
import { keyReadGrantSchema, type KeyReadGrantFormData } from "@/schemas/sa-key-read-grant";
import { useAuthStore } from "@/stores/auth-store";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Form, FormControl, FormField, FormItem, FormLabel, FormMessage, useAppForm } from "@/components/ui/form";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";

export function KeyReadGrantSection({ saId }: { readonly saId: string }) {
  const isAdmin = useAuthStore((state) => state.user?.is_admin ?? false);
  const { data: grant, isLoading, error } = useSaKeyReadGrant(saId);
  const save = useSaveSaKeyReadGrant(saId);
  const revoke = useRevokeSaKeyReadGrant(saId);
  const [editing, setEditing] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const form = useAppForm<KeyReadGrantFormData>({
    resolver: zodResolver(keyReadGrantSchema),
    mode: "onChange",
    defaultValues: { user_service_ids: "", expires_at: "" },
  });
  const { isDirty, isValid } = form.formState;

  async function submit(values: KeyReadGrantFormData) {
    setSubmitError(null);
    try {
      await save.mutateAsync({
        user_service_ids: values.user_service_ids.split(/[,\s]+/).filter(Boolean),
        expires_at: values.expires_at ? new Date(values.expires_at).toISOString() : undefined,
      });
      setEditing(false);
      toast.success("Connection metadata access granted");
    } catch (error) {
      setSubmitError(error instanceof ApiError ? error.message : "Could not save key read grant");
    }
  }

  async function revokeGrant() {
    try {
      await revoke.mutateAsync();
      toast.success("Connection metadata access revoked");
    } catch (error) {
      toast.error(error instanceof ApiError ? error.message : "Could not revoke key read grant");
    }
  }

  return (
    <DetailSection title="Connection metadata access">
      <div className="space-y-4 p-5">
        <p className="text-[12px] text-muted-foreground">
          Optional access to selected private connections owned by a person or
          organization. Catalog editors read the platform catalog through their
          assigned roles and do not need this grant. For private connection
          reads, the account needs user-services:read and a platform
          administrator must grant the connection UUIDs below. Connections must
          remain accessible to the account owner.
        </p>
        {isLoading && <p>Loading grant…</p>}
        {error && <p role="alert">Could not load connection metadata access.</p>}
        {!isLoading && !error && (
          <>
            <DetailRow label="Connection UUIDs" value={grant?.user_service_ids.join(", ") ?? "None"} />
            {grant && <DetailRow label="Expires" value={grant.expires_at ? formatDate(grant.expires_at) : "No expiry"} />}
            {isAdmin && !editing && (
              <div className="flex justify-end gap-2">
                {grant && <Button variant="destructive" isLoading={revoke.isPending} onClick={() => void revokeGrant()}>Revoke read access</Button>}
                <Button onClick={() => {
                  form.reset({ user_service_ids: grant?.user_service_ids.join(", ") ?? "", expires_at: grant?.expires_at ?? "" });
                  setSubmitError(null);
                  setEditing(true);
                }}>{grant ? "Replace read grant" : "Grant read access"}</Button>
              </div>
            )}
          </>
        )}
        {isAdmin && editing && (
          <Form {...form}>
            <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
              {([
                ["user_service_ids", "User-service UUIDs", "Comma-separated connection UUIDs"],
                ["expires_at", "Expiry (optional)", "2026-12-31T23:59:59Z"],
              ] as const).map(([name, label, placeholder]) => (
                <FormField key={name} control={form.control} name={name} render={({ field }) => (
                  <FormItem><FormLabel>{label}</FormLabel><FormControl><Input {...field} placeholder={placeholder} /></FormControl><FormMessage /></FormItem>
                )} />
              ))}
              {submitError && <p role="alert" className="text-destructive">{submitError}</p>}
              <div className="flex justify-end gap-2">
                <Button type="button" variant="outline" onClick={() => setEditing(false)}>Cancel</Button>
                <Button type="submit" isLoading={save.isPending} disabled={!isDirty || !isValid}>Save read grant</Button>
              </div>
            </form>
          </Form>
        )}
      </div>
    </DetailSection>
  );
}
