import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import {
  useAdminManagedLineActions,
  useAdminManagedLines,
} from "@/hooks/use-admin-managed-lines";
import {
  UNDERWRITING_WORDING,
  evidenceFormSchema,
  underwritingFormSchema,
  voiceAccountFormSchema,
  type EvidenceForm,
  type ManagedAccount,
  type UnderwritingForm,
  type VoiceAccountForm,
} from "@/schemas/admin-managed-lines";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { UnreadableRecords } from "./unreadable-records";
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
import { failure, when } from "./format";

export function AccountsSection() {
  useUiTranslation();
  const { accounts, premiumEnabled } = useAdminManagedLines();
  const actions = useAdminManagedLineActions();
  const [rotating, setRotating] = useState<ManagedAccount>();
  const [underwriting, setUnderwriting] = useState<ManagedAccount>();
  const [voice, setVoice] = useState<ManagedAccount>();
  const [error, setError] = useState<string>();
  async function premium(on: boolean) {
    setError(undefined);
    try {
      await actions.premiumSwitch.mutateAsync(on);
    } catch (cause) {
      setError(failure(cause, translateUi("Could not change the premium voice switch.")));
    }
  }
  return (
    <div className="space-y-3">
      <h3 className="font-medium">{translateUi("Managed line accounts")}</h3>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border p-3">
        <Label className="flex-1" htmlFor="managed-premium-switch">
          {translateUi("Premium ElevenLabs voices (global switch)")}
          <span className="block text-muted-foreground">{translateUi("Turning this off stops premium synthesis at the next block: live premium calls speak their wrap-up and end. New premium claims and selections are refused.")}</span>
        </Label>
        <Switch
          id="managed-premium-switch"
          checked={premiumEnabled}
          disabled={actions.premiumSwitch.isPending}
          onCheckedChange={(on) => void premium(on)}
        />
      </div>
      {accounts.error && (
        <ErrorBanner
          message={translateUi("Could not load managed line accounts.")}
          onRetry={() => void accounts.refetch()}
        />
      )}
      <UnreadableRecords ids={accounts.unreadableIds} />
      {error && <ErrorBanner message={error} />}
      {accounts.data?.map((a) => (
        <article key={a.id} className="space-y-2 rounded-lg border border-border p-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p>
              {a.provider} · {a.account_id}
            </p>
            {a.provider === "telnyx" && (
              <Badge variant={a.underwriting?.enabled ? "success" : "secondary"}>
                {a.underwriting?.enabled
                  ? translateUi("Underwriting enabled")
                  : translateUi("Telnyx voice refused")}
              </Badge>
            )}
          </div>
          {a.underwriting && (
            <p className="text-muted-foreground">
              {translateUi("Underwriting revision {{revision}} · maximum leg {{seconds}} s · rolling 30-day budget {{budget}} · accepted {{time}}", {
                revision: a.underwriting.revision,
                seconds: a.underwriting.max_leg_seconds,
                budget: a.underwriting.rolling_budget,
                time: when(a.underwriting.accepted_at),
              })}
            </p>
          )}
          {a.voice && (
            <p className="text-muted-foreground">
              {translateUi("Voice revision {{revision}} · {{characters}} characters per call · premium {{premium}}", {
                revision: a.voice.revision,
                characters: a.voice.per_call_characters,
                premium: a.voice.premium ? a.voice.premium.plan_tier : translateUi("not configured"),
              })}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => setRotating(a)}>
              {translateUi("Rotate credential")}
            </Button>
            {a.provider === "telnyx" && (
              <>
                <Button variant="outline" onClick={() => setUnderwriting(a)}>
                  {translateUi("Underwriting")}
                </Button>
                <Button variant="outline" onClick={() => setVoice(a)}>
                  {translateUi("Speech and premium voice")}
                </Button>
              </>
            )}
          </div>
        </article>
      ))}
      {accounts.hasNextPage && (
        <Button
          variant="outline"
          disabled={accounts.isFetchingNextPage}
          onClick={() => void accounts.fetchNextPage()}
        >
          {translateUi("Load more accounts")}
        </Button>
      )}
      {rotating && <RotateDialog account={rotating} onClose={() => setRotating(undefined)} />}
      {underwriting && (
        <UnderwritingDialog account={underwriting} onClose={() => setUnderwriting(undefined)} />
      )}
      {voice && <VoiceDialog account={voice} onClose={() => setVoice(undefined)} />}
      <EvidenceSection />
    </div>
  );
}

function RotateDialog({ account, onClose }: { readonly account: ManagedAccount; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [credential, setCredential] = useState("");
  const [error, setError] = useState<string>();
  async function save() {
    setError(undefined);
    try {
      await actions.rotate.mutateAsync({
        accountId: account.id,
        revision: account.revision,
        credential,
      });
      setCredential("");
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Rotation was refused. The previous credential remains active.")));
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          setCredential("");
          onClose();
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Rotate {{account}}", { account: account.account_id })}</DialogTitle>
          <DialogDescription>{translateUi("NyxID proves with provider reads that the new credential is the same provider account and can read every line and unfinished effect before it replaces the stored credential. The value is never shown again.")}</DialogDescription>
        </DialogHeader>
        <Label className="block space-y-1">
          <span>{translateUi("New provider credential")}</span>
          <Input
            type="password"
            autoComplete="off"
            value={credential}
            onChange={(event) => setCredential(event.target.value)}
          />
        </Label>
        {error && <ErrorBanner message={error} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {translateUi("Cancel")}
          </Button>
          <Button disabled={!credential || actions.rotate.isPending} onClick={() => void save()}>
            {actions.rotate.isPending ? translateUi("Proving…") : translateUi("Prove and rotate")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function UnderwritingDialog({ account, onClose }: { readonly account: ManagedAccount; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const u = account.underwriting;
  const form = useAppForm<UnderwritingForm>({
    resolver: zodResolver(underwritingFormSchema),
    mode: "onChange",
    defaultValues: {
      enabled: u?.enabled ?? false,
      acceptance_text: u?.acceptance_text || UNDERWRITING_WORDING,
      acceptance_version: u?.acceptance_version ?? "",
      paid_tier_evidence_reference: u?.paid_tier_evidence_reference ?? "",
      max_leg_seconds: u?.max_leg_seconds ?? 0,
      ceiling_kind: "written_confirmation",
      ceiling_reference: u?.max_leg_evidence_reference ?? "",
      hangup_cause: "",
      paid_tier: false,
      timestamp_fixture_reference: u?.timestamp_fixture_reference ?? "",
      detail_record_options_reference: u?.detail_record_options_reference ?? "",
      detail_record_fixture_reference: u?.detail_record_fixture_reference ?? "",
      detail_record_fields: (u?.detail_record_fields ?? ["call_leg_id", "connection_id", "answered_at", "finished_at"]).join(", "),
      supplier_tariff_revision: u?.supplier_tariff_revision ?? 1,
      fx_policy_revision: u?.fx_policy_revision ?? "",
      character_tariff_reference: u?.character_tariff_reference ?? "",
      components: JSON.stringify(u?.components ?? [], null, 2),
      rolling_budget: u?.rolling_budget ?? "",
    },
  });
  const ceiling = form.watch("ceiling_kind");
  async function save(v: UnderwritingForm) {
    setError(undefined);
    try {
      // Exact decimal credit strings, validated by the form schema.
      const components = JSON.parse(v.components) as unknown;
      await actions.underwriting.mutateAsync({
        accountId: account.id,
        body: {
          revision: u?.revision ?? 0,
          enabled: v.enabled,
          acceptance_text: v.acceptance_text,
          acceptance_version: v.acceptance_version,
          paid_tier_evidence_reference: v.paid_tier_evidence_reference,
          max_leg_seconds: v.max_leg_seconds,
          ceiling_evidence:
            v.ceiling_kind === "written_confirmation"
              ? { kind: "written_confirmation", reference: v.ceiling_reference }
              : {
                  kind: "paid_inbound_test",
                  reference: v.ceiling_reference,
                  hangup_cause: v.hangup_cause,
                  paid_tier: v.paid_tier,
                },
          timestamp_fixture_reference: v.timestamp_fixture_reference || null,
          detail_record_options_reference: v.detail_record_options_reference,
          detail_record_fixture_reference: v.detail_record_fixture_reference,
          detail_record_fields: v.detail_record_fields
            .split(",")
            .map((f) => f.trim())
            .filter(Boolean),
          supplier_tariff_revision: v.supplier_tariff_revision,
          fx_policy_revision: v.fx_policy_revision,
          character_tariff_reference: v.character_tariff_reference,
          components,
          rolling_budget: v.rolling_budget,
        },
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Underwriting was refused; the previous policy remains.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Telnyx underwriting for {{account}}", { account: account.account_id })}</DialogTitle>
          <DialogDescription>{translateUi("Underwritten exception, not reserved liability. Telnyx cannot cap an inbound leg, so NyxID absorbs supplier cost beyond the 1,800-second retail limit up to this rolling budget. Every item is required; missing evidence keeps managed Telnyx voice refused.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            <CheckField control={form.control} name="enabled" label={translateUi("Enable managed Telnyx voice on this account")} />
            <TextField control={form.control} name="acceptance_text" label={translateUi("Commercial acceptance text")} multiline />
            <TextField control={form.control} name="acceptance_version" label={translateUi("Acceptance version")} />
            <TextField control={form.control} name="paid_tier_evidence_reference" label={translateUi("Paid-tier account evidence")} />
            <NumberField control={form.control} name="max_leg_seconds" label={translateUi("Maximum inbound leg (seconds)")} />
            <ChoiceField
              control={form.control}
              name="ceiling_kind"
              label={translateUi("Maximum leg evidence")}
              choices={[
                { value: "written_confirmation", label: translateUi("Written Telnyx confirmation") },
                { value: "paid_inbound_test", label: translateUi("Observed paid inbound test call") },
              ]}
            />
            <TextField control={form.control} name="ceiling_reference" label={translateUi("Evidence reference")} />
            {ceiling === "paid_inbound_test" && (
              <>
                <TextField control={form.control} name="hangup_cause" label={translateUi("Observed hangup cause")} help={translateUi("Must be time_limit. Trial-account evidence is invalid.")} />
                <CheckField control={form.control} name="paid_tier" label={translateUi("The test call used a paid-tier account")} />
              </>
            )}
            <TextField control={form.control} name="timestamp_fixture_reference" label={translateUi("Answered/hangup timestamp fixture (optional)")} />
            <TextField control={form.control} name="detail_record_options_reference" label={translateUi("Detail-record options evidence")} />
            <TextField control={form.control} name="detail_record_fixture_reference" label={translateUi("Detail-record fixture")} />
            <TextField control={form.control} name="detail_record_fields" label={translateUi("Verified detail-record fields")} help={translateUi("Comma-separated; call_leg_id, connection_id, answered_at and finished_at enable detail-record evidence.")} />
            <NumberField control={form.control} name="supplier_tariff_revision" label={translateUi("Supplier tariff revision")} />
            <TextField control={form.control} name="fx_policy_revision" label={translateUi("Frozen FX policy revision")} />
            <TextField control={form.control} name="character_tariff_reference" label={translateUi("Character-priced synthesis tariff")} />
            <TextField
              control={form.control}
              name="components"
              label={translateUi("Per-second exposure components (JSON)")}
              help={translateUi("Carrier inbound, relay and speech-to-text credits per second with supplier increments and minimums.")}
              multiline
            />
            <TextField control={form.control} name="rolling_budget" label={translateUi("Rolling 30-day budget (credits)")} help={translateUi("Saving is refused when registered channel limits times the worst-case exposure exceed this budget.")} />
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button type="submit" disabled={!form.formState.isValid || actions.underwriting.isPending}>
                {translateUi("Accept and save")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function VoiceDialog({ account, onClose }: { readonly account: ManagedAccount; readonly onClose: () => void }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const v0 = account.voice;
  const p = v0?.premium;
  const form = useAppForm<VoiceAccountForm>({
    resolver: zodResolver(voiceAccountFormSchema),
    mode: "onChange",
    defaultValues: {
      per_call_characters: v0?.per_call_characters ?? 12000,
      greeting: v0?.greeting ?? "",
      wrapup: v0?.wrapup ?? "",
      premium: !!p,
      api_key_ref: p?.api_key_ref ?? "",
      voices: JSON.stringify(p?.voices ?? [], null, 2),
      billing_account_identity: p?.billing_account_identity ?? "",
      monthly_characters: p?.monthly_characters ?? 0,
      plan_tier: (p?.plan_tier as VoiceAccountForm["plan_tier"] | undefined) ?? "Scale",
      oem_terms_version: p?.oem_terms_version ?? "",
      admin_acknowledged: false,
      billing_service_id: p?.billing_service_id ?? "",
    },
  });
  const premium = form.watch("premium");
  async function save(v: VoiceAccountForm) {
    setError(undefined);
    try {
      await actions.voiceAccount.mutateAsync({
        accountId: account.id,
        body: {
          revision: v0?.revision ?? 0,
          per_call_characters: v.per_call_characters,
          greeting: v.greeting || null,
          wrapup: v.wrapup || null,
          premium: v.premium
            ? {
                api_key_ref: v.api_key_ref,
                voices: JSON.parse(v.voices) as unknown,
                billing_account_identity: v.billing_account_identity,
                monthly_characters: v.monthly_characters,
                plan_tier: v.plan_tier,
                oem_terms_version: v.oem_terms_version,
                admin_acknowledged: v.admin_acknowledged,
                billing_service_id: v.billing_service_id,
              }
            : null,
        },
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Voice configuration was refused.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Speech for {{account}}", { account: account.account_id })}</DialogTitle>
          <DialogDescription>{translateUi("Every Telnyx managed call has a character allowance debited in blocks before text is spoken; 300 characters are reserved for the wrap-up. NyxID speaks no greeting unless you configure one.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            <NumberField control={form.control} name="per_call_characters" label={translateUi("Characters per call")} />
            <TextField control={form.control} name="greeting" label={translateUi("Greeting (optional)")} multiline />
            <TextField control={form.control} name="wrapup" label={translateUi("Wrap-up override (optional, at most 300 characters)")} help={translateUi("Leave empty to use the built-in wrap-up in the line’s language.")} multiline />
            <CheckField control={form.control} name="premium" label={translateUi("Offer premium ElevenLabs voices on this account")} />
            {premium && (
              <>
                <TextField control={form.control} name="api_key_ref" label={translateUi("Telnyx integration secret identifier")} help={translateUi("The ElevenLabs key stays in Telnyx; NyxID proves the identifier exists and stores only the reference.")} />
                <TextField control={form.control} name="voices" label={translateUi("Allowed premium voices (JSON)")} help={translateUi("ElevenLabs.<model>.<voice_id> with en-US, zh-CN or zh-TW.")} multiline />
                <TextField control={form.control} name="billing_account_identity" label={translateUi("ElevenLabs billing account identity")} />
                <NumberField control={form.control} name="monthly_characters" label={translateUi("Monthly character budget (shared by this ElevenLabs account)")} />
                <ChoiceField
                  control={form.control}
                  name="plan_tier"
                  label={translateUi("ElevenLabs plan tier")}
                  choices={["Scale", "Business", "Enterprise"].map((value) => ({ value, label: value }))}
                />
                <TextField control={form.control} name="oem_terms_version" label={translateUi("OEM terms version")} help={translateUi("Merchants must accept an End User Agreement with this version before premium voices.")} />
                <ServiceField control={form.control} name="billing_service_id" label={translateUi("Premium billing service")} />
                <CheckField control={form.control} name="admin_acknowledged" label={translateUi("I confirm the ElevenLabs plan and OEM minimum terms are in place for this account.")} />
              </>
            )}
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button type="submit" disabled={!form.formState.isValid || actions.voiceAccount.isPending}>
                {translateUi("Save speech settings")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

/** The only manual quantity path: permanently missing provider evidence. */
export function EvidenceSection({ sessionId = "" }: { readonly sessionId?: string }) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [error, setError] = useState<string>();
  const [done, setDone] = useState(false);
  const form = useAppForm<EvidenceForm>({
    resolver: zodResolver(evidenceFormSchema),
    mode: "onChange",
    defaultValues: { session_id: sessionId, provider_seconds: 0, provider_reference: "" },
  });
  async function save(v: EvidenceForm) {
    setError(undefined);
    setDone(false);
    try {
      await actions.resolveEvidence.mutateAsync({
        sessionId: v.session_id,
        seconds: v.provider_seconds,
        reference: v.provider_reference,
      });
      setDone(true);
      form.reset({ session_id: "", provider_seconds: 0, provider_reference: "" });
    } catch (cause) {
      setError(failure(cause, translateUi("Evidence resolution was refused.")));
    }
  }
  return (
    <details className="rounded-lg border border-border p-3">
      <summary className="cursor-pointer">{translateUi("Resolve missing call evidence")}</summary>
      <p className="mt-2 text-muted-foreground">{translateUi("Use only when provider call duration is permanently unavailable for a terminated call. The provider reference and seconds are frozen and audited; billing uses at most 1,800 seconds.")}</p>
      <Form {...form}>
        <form className="mt-3 space-y-3" onSubmit={form.handleSubmit(save)}>
          <TextField control={form.control} name="session_id" label={translateUi("Call session ID")} />
          <NumberField control={form.control} name="provider_seconds" label={translateUi("Provider seconds")} />
          <TextField control={form.control} name="provider_reference" label={translateUi("Provider evidence reference")} />
          {error && <ErrorBanner message={error} />}
          {done && <p role="status">{translateUi("Evidence recorded and the call settled.")}</p>}
          <Button type="submit" disabled={!form.formState.isValid || actions.resolveEvidence.isPending}>
            {translateUi("Resolve evidence")}
          </Button>
        </form>
      </Form>
    </details>
  );
}
