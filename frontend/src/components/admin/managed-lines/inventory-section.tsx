import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import type { Control } from "react-hook-form";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import {
  useAdminManagedLineActions,
  useAdminManagedLines,
} from "@/hooks/use-admin-managed-lines";
import {
  costPolicyFormSchema,
  lineRegistrationFormSchema,
  lineUpdateFormSchema,
  type AdminManagedLine,
  type CostPolicyForm,
  type LineRegistrationForm,
  type LineUpdateForm,
  type ManagedLineOperation,
  type ManagedLinePreparation,
  rfc3339,
} from "@/schemas/admin-managed-lines";
import { newRequestId } from "@/schemas/managed-lines";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  CheckField,
  ChoiceField,
  NumberField,
  ServiceField,
  TextField,
} from "./fields";
import { failure, lineStateLabel, when } from "./format";
import { UnreadableRecords } from "./unreadable-records";

type Confirm =
  | { kind: "suspend" | "resume" | "approve"; line: AdminManagedLine }
  | { kind: "release"; line: AdminManagedLine; requestId: string }
  | { kind: "retire"; line: AdminManagedLine; releasedAt: string };

const TARIFF_EXAMPLE = JSON.stringify(
  [
    {
      kind: "carrier_inbound",
      amount: "0.0085",
      currency: "USD",
      unit: "minute",
      increment: 60,
      minimum: 60,
      source: "https://supplier.example/pricing",
      revision: 1,
      fx_policy_revision: "fx-2026-11",
      effective_at: "2026-11-01T00:00:00Z",
    },
  ],
  null,
  2,
);
const tariffJson = (line: AdminManagedLine) =>
  JSON.stringify(
    line.supplier_tariffs.map((t) => ({ ...t })),
    null,
    2,
  );

export function InventorySection() {
  useUiTranslation();
  const { lines, accounts } = useAdminManagedLines();
  const actions = useAdminManagedLineActions();
  const [prepared, setPrepared] = useState<ManagedLinePreparation>();
  const [editing, setEditing] = useState<AdminManagedLine>();
  const [voices, setVoices] = useState<AdminManagedLine>();
  const [policy, setPolicy] = useState<AdminManagedLine>();
  const [confirm, setConfirm] = useState<Confirm>();
  const [result, setResult] = useState<ManagedLineOperation>();
  const [error, setError] = useState<string>();
  async function act(task: () => Promise<unknown>) {
    setError(undefined);
    try {
      await task();
    } catch (cause) {
      setError(failure(cause, translateUi("Managed line action failed.")));
    }
  }
  async function decide() {
    if (!confirm) return;
    const { line } = confirm;
    const body = { revision: line.revision };
    await act(async () => {
      switch (confirm.kind) {
        case "suspend":
          setResult(await actions.suspend.mutateAsync({ lineId: line.id, body }));
          break;
        case "resume":
          setResult(await actions.resume.mutateAsync({ lineId: line.id, body }));
          break;
        case "retire":
          setResult(
            await actions.retire.mutateAsync({
              lineId: line.id,
              body: {
                ...body,
                supplier_released_at: confirm.releasedAt
                  ? new Date(confirm.releasedAt).toISOString()
                  : null,
              },
            }),
          );
          break;
        case "approve":
          await actions.approve.mutateAsync({ lineId: line.id, revision: line.revision });
          break;
        case "release":
          await actions.release.mutateAsync({
            request_id: confirm.requestId,
            line_id: line.id,
            claim_id: line.claim!.id,
          });
          break;
      }
      setConfirm(undefined);
    });
  }
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="font-medium">{translateUi("NyxID-managed lines")}</h3>
        <Button
          variant="outline"
          disabled={actions.prepare.isPending}
          onClick={() => void act(async () => setPrepared(await actions.prepare.mutateAsync()))}
        >
          {translateUi("Register a line")}
        </Button>
      </div>
      <p className="text-muted-foreground">{translateUi("Lines are NyxID-owned numbers in platform custody. Registration and edits prove the number, application and stable webhook URLs with the trusted account’s credential before saving. Merchants claim lines; custody never transfers.")}</p>
      {lines.error && (
        <ErrorBanner
          message={translateUi("Could not load managed lines.")}
          onRetry={() => void lines.refetch()}
        />
      )}
      <UnreadableRecords ids={lines.unreadableIds} />
      {error && <ErrorBanner message={error} />}
      {result && (
        <p role="status" className="text-muted-foreground">
          {result.termination_pending
            ? translateUi("{{state}}. Termination is pending for {{count}} live calls; recovery keeps requesting hangups until the provider confirms.", {
                state: lineStateLabel(result.state, result.suspension),
                count: result.live_calls,
              })
            : translateUi("Line is now {{state}}.", {
                state: lineStateLabel(result.state, result.suspension),
              })}
        </p>
      )}
      {lines.data?.map((line) => (
        <article key={line.id} className="space-y-2 rounded-lg border border-border p-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p>
              {line.phone_number} · {line.provider} · {line.country}
            </p>
            <Badge variant={line.state === "suspended" ? "warning" : line.state === "retired" ? "secondary" : "default"}>
              {lineStateLabel(line.state, line.suspension)}
            </Badge>
          </div>
          <p className="text-muted-foreground">
            {translateUi("Revision {{revision}} · {{capabilities}} · slots {{slots}} (provider {{provider}}) · {{calls}} calls and {{sms}} SMS per minute", {
              revision: line.revision,
              capabilities: [line.voice ? translateUi("Voice") : null, line.sms ? translateUi("SMS") : null].filter(Boolean).join(" + "),
              slots: line.call_slot_limit,
              provider: line.provider_channel_limit ?? "—",
              calls: line.calls_per_minute,
              sms: line.sms_per_minute,
            })}
          </p>
          {line.claim && (
            <p className="text-muted-foreground">
              {translateUi("Claim {{claim}} · owner {{owner}} · {{state}} · covered until {{until}} · {{live}} live calls", {
                claim: line.claim.id,
                owner: line.claim.owner_id,
                state: lineStateLabel(line.claim.state, line.claim.suspension),
                until: when(line.claim.covered_until),
                live: line.live_calls,
              })}
            </p>
          )}
          {line.available_at && (
            <p className="text-muted-foreground">
              {translateUi("Cooldown ends {{time}}", { time: when(line.available_at) })}
            </p>
          )}
          {line.supplier_released_at && (
            <p className="text-muted-foreground">
              {translateUi("Released by the supplier {{time}}", { time: when(line.supplier_released_at) })}
            </p>
          )}
          <details>
            <summary className="cursor-pointer">{translateUi("Webhook URLs and evidence")}</summary>
            <dl className="mt-2 space-y-1 break-all font-mono">
              <dt>{translateUi("Voice URL")}</dt>
              <dd>{line.voice_url}</dd>
              <dt>{translateUi("SMS URL")}</dt>
              <dd>{line.sms_url}</dd>
              <dt>{translateUi("Regulatory")}</dt>
              <dd>
                {line.regulatory.requirement_reference} · {line.regulatory.registration_status} · {when(line.regulatory.verified_at)}
              </dd>
              <dt>{translateUi("Cost policy revision")}</dt>
              <dd>{line.cost_policy_revision ?? "—"}</dd>
            </dl>
          </details>
          <div className="flex flex-wrap gap-2">
            {line.state !== "retired" && (
              <Button variant="outline" onClick={() => setEditing(line)}>
                {translateUi("Edit and revalidate")}
              </Button>
            )}
            {line.voice && line.state !== "retired" && (
              <Button variant="outline" onClick={() => setVoices(line)}>
                {translateUi("Standard voices")}
              </Button>
            )}
            <Button variant="outline" onClick={() => setPolicy(line)}>
              {translateUi("Cost policy")}
            </Button>
            {(line.state === "available" || line.state === "claimed") && (
              <Button variant="outline" onClick={() => setConfirm({ kind: "suspend", line })}>
                {translateUi("Suspend")}
              </Button>
            )}
            {line.state === "suspended" && line.suspension === "admin" && (
              <Button variant="outline" onClick={() => setConfirm({ kind: "resume", line })}>
                {translateUi("Resume")}
              </Button>
            )}
            {line.state === "pending_reassignment" && (
              <Button variant="outline" onClick={() => setConfirm({ kind: "approve", line })}>
                {translateUi("Approve reassignment")}
              </Button>
            )}
            {line.claim && (line.claim.state === "claimed" || line.claim.state === "suspended") && (
              <Button
                variant="outline"
                onClick={() => setConfirm({ kind: "release", line, requestId: newRequestId() })}
              >
                {translateUi("Release claim")}
              </Button>
            )}
            {!line.claim && (line.state !== "retired" || !line.supplier_released_at) && (
              <Button
                variant="outline"
                onClick={() => setConfirm({ kind: "retire", line, releasedAt: "" })}
              >
                {line.state === "retired"
                  ? translateUi("Record supplier release")
                  : translateUi("Retire")}
              </Button>
            )}
          </div>
        </article>
      ))}
      {lines.hasNextPage && (
        <Button
          variant="outline"
          disabled={lines.isFetchingNextPage}
          onClick={() => void lines.fetchNextPage()}
        >
          {translateUi("Load more lines")}
        </Button>
      )}
      {prepared && (
        <RegisterDialog
          prepared={prepared}
          accounts={(accounts.data ?? []).map((a) => ({
            value: a.id,
            label: `${a.provider} · ${a.account_id}`,
          }))}
          onClose={() => setPrepared(undefined)}
        />
      )}
      {editing && <EditDialog line={editing} onClose={() => setEditing(undefined)} />}
      {voices && <VoicesDialog line={voices} onClose={() => setVoices(undefined)} />}
      {policy && <CostPolicyDialog line={policy} onClose={() => setPolicy(undefined)} />}
      <Dialog open={!!confirm} onOpenChange={(open) => !open && setConfirm(undefined)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{confirm ? confirmTitle(confirm) : null}</DialogTitle>
            <DialogDescription>{confirm ? confirmBody(confirm) : null}</DialogDescription>
          </DialogHeader>
          {confirm?.kind === "retire" && (
            <Label className="block space-y-1">
              <span>{translateUi("Supplier release time (optional)")}</span>
              <Input
                type="datetime-local"
                value={confirm.releasedAt}
                onChange={(event) => setConfirm({ ...confirm, releasedAt: event.target.value })}
              />
            </Label>
          )}
          {confirm?.kind === "release" && (
            <p className="font-mono text-muted-foreground">
              {translateUi("Request ID {{id}}", { id: confirm.requestId })}
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirm(undefined)}>
              {translateUi("Cancel")}
            </Button>
            <Button
              variant={confirm?.kind === "resume" || confirm?.kind === "approve" ? "primary" : "destructive"}
              disabled={
                actions.suspend.isPending ||
                actions.resume.isPending ||
                actions.retire.isPending ||
                actions.approve.isPending ||
                actions.release.isPending ||
                (confirm?.kind === "retire" && confirm.line.state === "retired" && !confirm.releasedAt)
              }
              onClick={() => void decide()}
            >
              {translateUi("Confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function confirmTitle(c: Confirm): string {
  switch (c.kind) {
    case "suspend":
      return translateUi("Suspend {{number}}?", { number: c.line.phone_number });
    case "resume":
      return translateUi("Resume {{number}}?", { number: c.line.phone_number });
    case "approve":
      return translateUi("Approve reassignment of {{number}}?", { number: c.line.phone_number });
    case "release":
      return translateUi("Release the claim on {{number}}?", { number: c.line.phone_number });
    case "retire":
      return c.line.state === "retired"
        ? translateUi("Record the supplier release of {{number}}?", { number: c.line.phone_number })
        : translateUi("Retire {{number}}?", { number: c.line.phone_number });
  }
}
function confirmBody(c: Confirm): string {
  switch (c.kind) {
    case "suspend":
      return translateUi("New calls and messages are refused, bindings are disabled and live calls are asked to hang up. The merchant’s paid period keeps running and is not renewed while suspended.");
    case "resume":
      return translateUi("A claim whose paid coverage ended during suspension becomes unpaid, with a fresh seven-day retention window; nothing is back-charged.");
    case "approve":
      return translateUi("The number has cooled down for at least 30 days. Approval returns it to available inventory.");
    case "release":
      return translateUi("Calls stop and the claim drains to cooldown once every effect is accounted. The merchant’s paid period is not refunded. Retrying uses the same request ID.");
    case "retire":
      return c.line.state === "retired"
        ? translateUi("Records when the supplier stopped billing this number. Inventory cost capture stops at that time. This is recorded once.")
        : translateUi("Retirement is local and permanent: the line never returns to inventory and stays in platform custody. Optionally record the supplier release now.");
  }
}

function lineBody(v: LineUpdateForm) {
  return {
    regulatory: {
      requirement_reference: v.requirement_reference,
      registration_status: v.registration_status,
      sms_sender_registration: v.sms_sender_registration || null,
    },
    supplier_tariffs: JSON.parse(v.supplier_tariffs) as unknown,
    call_slot_limit: v.call_slot_limit,
    calls_per_minute: v.calls_per_minute,
    sms_per_minute: v.sms_per_minute,
  };
}

function OperationsFields({ control }: { readonly control: Control<LineUpdateForm> }) {
  useUiTranslation();
  return (
    <>
      <TextField control={control} name="requirement_reference" label={translateUi("Regulatory requirement or bundle reference")} />
      <TextField control={control} name="registration_status" label={translateUi("Regulatory registration status")} help={translateUi("Only approved registrations can be saved.")} />
      <TextField control={control} name="sms_sender_registration" label={translateUi("SMS sender registration reference")} />
      <TextField
        control={control}
        name="supplier_tariffs"
        label={translateUi("Supplier tariffs (JSON)")}
        help={translateUi("Amount, currency, unit, increment, minimum, effective date and an HTTPS source for each supplier charge. These are cost evidence, not retail prices.")}
        multiline
      />
      <NumberField control={control} name="call_slot_limit" label={translateUi("Call slot limit")} help={translateUi("Concurrent calls; the provider channel limit also applies.")} />
      <NumberField control={control} name="calls_per_minute" label={translateUi("Calls per minute")} />
      <NumberField control={control} name="sms_per_minute" label={translateUi("SMS per minute")} />
    </>
  );
}

function RegisterDialog({
  prepared,
  accounts,
  onClose,
}: {
  readonly prepared: ManagedLinePreparation;
  readonly accounts: readonly { readonly value: string; readonly label: string }[];
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const form = useAppForm<LineRegistrationForm>({
    resolver: zodResolver(lineRegistrationFormSchema),
    mode: "onChange",
    defaultValues: {
      line_id: prepared.line_id,
      trusted_account_id: "",
      phone_number: "",
      country: "",
      number_resource_id: "",
      voice_application_id: "",
      voice_billing_service_id: "",
      sms_application_id: "",
      sms_billing_service_id: "",
      rental_billing_service_id: "",
      standard_voice_options: "",
      requirement_reference: "",
      registration_status: "approved",
      sms_sender_registration: "",
      supplier_tariffs: TARIFF_EXAMPLE,
      call_slot_limit: 1,
      calls_per_minute: 10,
      sms_per_minute: 10,
    },
  });
  async function save(v: LineRegistrationForm) {
    setError(undefined);
    try {
      await actions.register.mutateAsync({
        line_id: v.line_id,
        trusted_account_id: v.trusted_account_id,
        phone_number: v.phone_number,
        country: v.country,
        number_resource_id: v.number_resource_id,
        voice: v.voice_application_id
          ? { application_id: v.voice_application_id, billing_service_id: v.voice_billing_service_id }
          : null,
        sms: v.sms_application_id
          ? { application_id: v.sms_application_id, billing_service_id: v.sms_billing_service_id }
          : null,
        standard_voice_options: v.standard_voice_options.trim()
          ? (JSON.parse(v.standard_voice_options) as unknown)
          : [],
        rental_billing_service_id: v.rental_billing_service_id,
        ...lineBody(v),
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Provider proof or registration failed.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Register a NyxID-managed line")}</DialogTitle>
          <DialogDescription>{translateUi("Configure these stable URLs as the provider application’s inbound URLs (POST) before registering. Leave Twilio voice fallback URLs empty. Telnyx voice also needs a non-empty inbound channel limit and enabled underwriting.")}</DialogDescription>
        </DialogHeader>
        <dl className="space-y-1 break-all font-mono">
          <dt>{translateUi("Line ID")}</dt>
          <dd>{prepared.line_id}</dd>
          <dt>{translateUi("Voice URL")}</dt>
          <dd>{prepared.voice_url}</dd>
          <dt>{translateUi("SMS URL")}</dt>
          <dd>{prepared.sms_url}</dd>
        </dl>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            <ChoiceField control={form.control} name="trusted_account_id" label={translateUi("Trusted provider account")} choices={accounts} placeholder={translateUi("Choose an account")} />
            <TextField control={form.control} name="phone_number" label={translateUi("Phone number (E.164)")} />
            <TextField control={form.control} name="country" label={translateUi("Country (ISO)")} />
            <TextField control={form.control} name="number_resource_id" label={translateUi("Provider number resource ID")} />
            <TextField control={form.control} name="voice_application_id" label={translateUi("Voice application or connection ID")} help={translateUi("Leave empty for an SMS-only number. Only advertise capabilities the number really has.")} />
            <ServiceField control={form.control} name="voice_billing_service_id" label={translateUi("Voice billing service")} />
            <TextField control={form.control} name="sms_application_id" label={translateUi("Messaging profile or application ID")} help={translateUi("Leave empty for a voice-only number.")} />
            <ServiceField control={form.control} name="sms_billing_service_id" label={translateUi("SMS billing service")} />
            <ServiceField control={form.control} name="rental_billing_service_id" label={translateUi("Rental billing service")} />
            <TextField
              control={form.control}
              name="standard_voice_options"
              label={translateUi("Standard voice and language allowlist (JSON, optional)")}
              help={translateUi("Must include the provider default; leave empty for the default only.")}
              multiline
            />
            {/* Registration extends the editable operations form with identical field names. */}
            <OperationsFields control={form.control as unknown as Control<LineUpdateForm>} />
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button type="submit" disabled={!form.formState.isValid || actions.register.isPending}>
                {actions.register.isPending ? translateUi("Proving…") : translateUi("Prove and register")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function EditDialog({ line, onClose }: { readonly line: AdminManagedLine; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const form = useAppForm<LineUpdateForm>({
    resolver: zodResolver(lineUpdateFormSchema),
    mode: "onChange",
    defaultValues: {
      requirement_reference: line.regulatory.requirement_reference,
      registration_status: "approved",
      sms_sender_registration: line.regulatory.sms_sender_registration ?? "",
      supplier_tariffs: tariffJson(line),
      call_slot_limit: line.call_slot_limit,
      calls_per_minute: line.calls_per_minute,
      sms_per_minute: line.sms_per_minute,
    },
  });
  async function save(v: LineUpdateForm) {
    setError(undefined);
    try {
      await actions.update.mutateAsync({ lineId: line.id, body: { revision: line.revision, ...lineBody(v) } });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Provider revalidation or update failed.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Edit {{number}}", { number: line.phone_number })}</DialogTitle>
          <DialogDescription>{translateUi("Saving proves the number, applications and stable URLs again, refreshes the Telnyx channel limit and rechecks underwriting capacity. The number, capabilities and billing services cannot change.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            <OperationsFields control={form.control} />
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button type="submit" disabled={!form.formState.isValid || actions.update.isPending}>
                {actions.update.isPending ? translateUi("Proving…") : translateUi("Revalidate and save")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function VoicesDialog({ line, onClose }: { readonly line: AdminManagedLine; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [value, setValue] = useState(JSON.stringify(line.standard_voice_options, null, 2));
  const [error, setError] = useState<string>();
  async function save() {
    setError(undefined);
    try {
      const options = JSON.parse(value) as unknown;
      if (!Array.isArray(options)) throw new Error(translateUi("Enter a JSON list."));
      await actions.voices.mutateAsync({ lineId: line.id, body: { revision: line.revision, options } });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not save the voice allowlist.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Standard voices for {{number}}", { number: line.phone_number })}</DialogTitle>
          <DialogDescription>{translateUi("Allowed standard voice and language pairs for this line. The list must include the provider default and keep any claimed voice.")}</DialogDescription>
        </DialogHeader>
        <Textarea
          aria-label={translateUi("Standard voice and language allowlist")}
          className="font-mono"
          rows={8}
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        {error && <ErrorBanner message={error} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {translateUi("Cancel")}
          </Button>
          <Button disabled={actions.voices.isPending} onClick={() => void save()}>
            {translateUi("Save voices")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CostPolicyDialog({ line, onClose }: { readonly line: AdminManagedLine; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const form = useAppForm<CostPolicyForm>({
    resolver: zodResolver(costPolicyFormSchema),
    mode: "onChange",
    defaultValues: {
      fx: "[]",
      schemas: "[]",
      inventory: false,
      cycle_anchor: "",
      amount: "",
      currency: "USD",
      source: "",
      fx_policy_revision: "",
    },
  });
  const inventory = form.watch("inventory");
  async function save(v: CostPolicyForm) {
    setError(undefined);
    try {
      await actions.costPolicy.mutateAsync({
        lineId: line.id,
        body: {
          revision: line.cost_policy_revision ?? 0,
          fx: JSON.parse(v.fx) as unknown,
          schemas: JSON.parse(v.schemas) as unknown,
          inventory: v.inventory
            ? {
                cycle: "calendar_month",
                cycle_anchor: rfc3339(v.cycle_anchor),
                amount: v.amount,
                currency: v.currency,
                source: v.source,
                fx_policy_revision: v.fx_policy_revision,
              }
            : null,
        },
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not save the cost policy.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Cost policy for {{number}}", { number: line.phone_number })}</DialogTitle>
          <DialogDescription>{translateUi("Each save is a new immutable revision. Frozen FX rates, attested detail-record schemas and the supplier rental cycle drive cost evidence and the margin report; they never move customer money.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            <TextField control={form.control} name="fx" label={translateUi("FX rates (JSON)")} help={translateUi("Credits per unit of each supplier currency, with a revision label.")} multiline />
            <TextField control={form.control} name="schemas" label={translateUi("Detail-record schemas (JSON)")} help={translateUi("Only schemas verified with the provider options endpoint and a recorded fixture.")} multiline />
            <CheckField control={form.control} name="inventory" label={translateUi("Record supplier rental per calendar-month cycle")} />
            {inventory && (
              <>
                <TextField control={form.control} name="cycle_anchor" label={translateUi("Cycle anchor (RFC 3339)")} />
                <TextField control={form.control} name="amount" label={translateUi("Rental amount per cycle")} />
                <TextField control={form.control} name="currency" label={translateUi("Currency")} />
                <TextField control={form.control} name="source" label={translateUi("HTTPS tariff source")} />
                <TextField control={form.control} name="fx_policy_revision" label={translateUi("FX policy revision")} />
              </>
            )}
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button type="submit" disabled={!form.formState.isValid || actions.costPolicy.isPending}>
                {translateUi("Save cost policy")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
