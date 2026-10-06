import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useServiceConcurrency, useUpdateServiceConcurrency } from "@/hooks/use-service-concurrency";
import { useAdminUsers } from "@/hooks/use-admin";
import { concurrencyFormSchema, type ConcurrencyForm, type ConcurrencyPolicy } from "@/schemas/service-concurrency";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { OrgPicker, PersonPicker } from "@/components/admin-credits/credit-pickers";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";

const emptyPolicy: ConcurrencyPolicy = { default_limit: null, users: [], orgs: [] };

export function ServiceConcurrency({ serviceId }: { readonly serviceId: string }) {
  const query = useServiceConcurrency(serviceId);
  return <section className="mt-6 rounded-lg border border-border p-4">
    <h2 className="text-sm font-medium">Concurrency limits</h2>
    <p className="mt-1 text-xs text-muted-foreground">Limit simultaneous requests per person across all connections and replicas. Streams and WebSockets count until they close. Service accounts count separately.</p>
    {query.isPending ? <p className="mt-3 text-xs">Loading limits…</p> : query.isError ? <p role="alert" className="mt-3 text-xs text-destructive">Could not load concurrency limits. <Button variant="outline" onClick={() => void query.refetch()}>Retry</Button></p> : <PolicyEditor serviceId={serviceId} policy={query.data.policy} />}
  </section>;
}

function LimitInput({ label, value, onChange }: {
  readonly label: string;
  readonly value: number | null;
  readonly onChange: (value: number | null) => void;
}) {
  return <label className="flex flex-wrap items-center gap-3 text-xs">
    <span className="min-w-32 flex-1 break-all">{label}</span>
    <Input className="h-8 w-28" aria-label={label} type="number" min={1} max={10000} step={1} placeholder="Unlimited"
      value={value ?? ""} onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))} />
    <span className="w-16 text-muted-foreground">{value === null ? "Unlimited" : "requests"}</span>
  </label>;
}

export function PolicyEditor({ serviceId, policy }: { readonly serviceId: string; readonly policy: ConcurrencyPolicy | null }) {
  const [baseline, setBaseline] = useState(policy);
  const mutation = useUpdateServiceConcurrency(serviceId);
  const people = useAdminUsers(1, 100, undefined, "person");
  const orgs = useAdminUsers(1, 100, undefined, "org");
  const form = useAppForm<ConcurrencyForm>({
    resolver: zodResolver(concurrencyFormSchema),
    defaultValues: { enabled: policy !== null, policy: policy ?? emptyPolicy },
  });
  const enabled = form.watch("enabled");
  const current = form.watch("policy");
  const stale = JSON.stringify(baseline) !== JSON.stringify(policy);
  const count = current.users.length + current.orgs.length;
  function select(kind: "users" | "orgs", ids: string[]) {
    const rows = ids.map((id) => current[kind].find((row) => row.id === id) ?? { id, limit: null });
    form.setValue(`policy.${kind}`, rows);
  }
  async function save(values: ConcurrencyForm) {
    try {
      const result = await mutation.mutateAsync({ policy: values.enabled ? values.policy : null });
      setBaseline(result.policy);
      form.reset({ enabled: result.policy !== null, policy: result.policy ?? emptyPolicy });
      toast.success("Concurrency limits saved");
    } catch {
      form.setError("root", { message: "Could not save concurrency limits. Try again." });
    }
  }
  return <Form {...form}><form className="mt-4 space-y-4" onSubmit={form.handleSubmit(save)}>
    {stale && <StaleFormNotice onReload={() => { setBaseline(policy); form.reset({ enabled: policy !== null, policy: policy ?? emptyPolicy }); }} />}
    <label className="flex items-center gap-2 text-xs"><Switch aria-label="Enable concurrency limits" checked={enabled} onCheckedChange={(value) => form.setValue("enabled", value)} />Enable concurrency limits</label>
    <p className="text-xs text-muted-foreground">Disabled means unlimited for everyone. Leave a limit blank for unlimited. Changes apply to new requests; running requests are allowed to finish.</p>
    {enabled && <>
      <LimitInput label="Default per-person limit" value={current.default_limit} onChange={(value) => form.setValue("policy.default_limit", value)} />
      <p className="text-xs text-muted-foreground">A user override wins. Otherwise, the highest limit among active organization memberships wins, with unlimited highest. Without an override, the default applies.</p>
      <p className="text-xs text-muted-foreground">{count}/500 override targets. New overrides start unlimited.</p>
      {(["users", "orgs"] as const).map((kind) => {
        const Picker = kind === "users" ? PersonPicker : OrgPicker;
        const names = kind === "users" ? people.data?.users : orgs.data?.users;
        return <div key={kind} className="space-y-3">
          <h3 className="text-xs font-medium">{kind === "users" ? "User overrides" : "Organization overrides"}</h3>
          <Picker selected={current[kind].map((row) => row.id)} onChange={(ids) => select(kind, ids)} />
          {current[kind].map((row, index) => {
            const user = names?.find((u) => u.id === row.id);
            const name = user?.display_name || user?.slug || user?.email || row.id;
            return <LimitInput key={row.id} label={`${name} limit`} value={row.limit}
              onChange={(value) => form.setValue(`policy.${kind}.${index}.limit`, value)} />;
          })}
        </div>;
      })}
    </>}
    {form.formState.errors.root && <p role="alert" className="text-xs text-destructive">{form.formState.errors.root.message}</p>}
    <FormSubmitErrors />
    <Button type="submit" disabled={!form.formState.isDirty || mutation.isPending || stale}>{mutation.isPending ? "Saving…" : "Save concurrency limits"}</Button>
  </form></Form>;
}
