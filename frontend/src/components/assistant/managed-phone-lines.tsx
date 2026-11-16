import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import type { Control, FieldPath, FieldValues } from "react-hook-form";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { formatCreditAmount } from "@/lib/credits";
import { useAuthStore } from "@/stores/auth-store";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import {
  useManagedEligibility,
  useManagedLineActions,
  useManagedLines,
} from "@/hooks/use-managed-lines";
import {
  isPremiumVoice,
  managedClaimFormSchema,
  managedCoverageFormSchema,
  managedTermsFormSchema,
  newRequestId,
  perMinute,
  periodRental,
  termsAccepted,
  voiceKey,
  type ManagedClaimForm,
  type ManagedCoverageForm,
  type ManagedEligibility,
  type ManagedLine,
  type ManagedQuote,
  type ManagedRate,
  type ManagedTermsForm,
} from "@/schemas/managed-lines";
import {
  GroupedFooter,
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedLabel as FormLabel,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import { GroupedDialog, GroupedDialogContent } from "@/components/ui/grouped-dialog";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import { Form, FormControl, FormField, useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import {
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";

function when(value: string | null | undefined): string {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleString(getFormattingLocale());
}
function failure(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}
function stateLabel(state: string, suspension?: string | null): string {
  if (state === "suspended") {
    switch (suspension) {
      case "unpaid":
        return translateUi("Suspended: coverage ended");
      case "admin":
        return translateUi("Suspended by NyxID");
      case "eligibility_revoked":
        return translateUi("Suspended: eligibility revoked");
      default:
        return translateUi("Suspended");
    }
  }
  switch (state) {
    case "claimed":
      return translateUi("Active");
    case "releasing":
      return translateUi("Releasing");
    case "available":
      return translateUi("Available");
    default:
      return state;
  }
}
function rateText(rate: ManagedRate): string {
  const amount = formatCreditAmount(rate.credits_per_unit);
  switch (rate.purpose) {
    case "calls":
      return translateUi("Calls: {{amount}} per second ({{minute}} per minute)", {
        amount,
        minute: formatCreditAmount(perMinute(rate.credits_per_unit)),
      });
    case "premium_voice":
      return translateUi(
        "Premium voice: {{amount}} per second ({{minute}} per minute) in addition to calls",
        { amount, minute: formatCreditAmount(perMinute(rate.credits_per_unit)) },
      );
    case "sms":
      return translateUi("SMS: {{amount}} per message part, inbound and replies", {
        amount,
      });
    default:
      return translateUi("{{metric}}: {{amount}} per unit", {
        metric: rate.metric,
        amount,
      });
  }
}

export function ManagedPhoneLines() {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  return <OwnedManagedLines key={actor ?? "signed-out"} />;
}

function OwnedManagedLines() {
  useUiTranslation();
  const eligibility = useManagedEligibility();
  const lines = useManagedLines(eligibility.data != null);
  const [terms, setTerms] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [claiming, setClaiming] = useState<{ line: ManagedLine; quote: ManagedQuote }>();
  const e = eligibility.data;
  const accepted = e ? termsAccepted(e) : false;
  const claimed = lines.data?.filter((l) => l.claim) ?? [];
  const available = lines.data?.filter((l) => !l.claim && l.state === "available") ?? [];
  const active = claimed.filter((l) => l.claim?.state !== "cooldown").length;
  const canClaim = !!e && !e.revoked_at && accepted && active < e.max_lines;
  return (
    <section className="space-y-6" aria-label={translateUi("NyxID phone numbers")}>
      <GroupedList
        title={translateUi("NyxID phone numbers")}
        footer={
          <>
            <p>{translateUi("Get a NyxID-owned number answered by one of your specialists. NyxID keeps the carrier account, keys and webhooks; you pay for the line and its usage in NyxID credits.")}</p>
            <p>{translateUi("Line rental is prepaid for 30 days at a time and paid periods are not refunded. Calls and SMS use the rates shown in your quote.")}</p>
            {(eligibility.error || lines.error) && (
              <ErrorBanner
                message={translateUi("Could not load NyxID phone numbers.")}
                onRetry={() => {
                  void eligibility.refetch();
                  void lines.refetch();
                }}
              />
            )}
          </>
        }
      >
        {eligibility.isPending && (
          <GroupedRow title={translateUi("Loading phone numbers…")} role="status" />
        )}
        {eligibility.isSuccess && !e && (
          <GroupedRow
            title={translateUi("Merchant approval required")}
            description={translateUi("NyxID numbers are available to approved businesses. Ask NyxID support to review your business; approval cannot be requested here.")}
            accessory={<Badge variant="warning">{translateUi("Not approved")}</Badge>}
          />
        )}
        {e && (
          <GroupedRow
            title={translateUi("Merchant eligibility")}
            description={`${e.business_legal_name} · ${e.jurisdiction}`}
            accessory={
              e.revoked_at ? (
                <Badge variant="destructive">{translateUi("Revoked")}</Badge>
              ) : (
                <Badge variant="success">{translateUi("Approved")}</Badge>
              )
            }
          />
        )}
        {e && !e.revoked_at && (
          <GroupedRow
            title={translateUi("Limits")}
            description={
              e.monthly_spend_limit
                ? translateUi("Up to {{count}} lines · monthly phone spend limit {{amount}}", {
                    count: e.max_lines,
                    amount: formatCreditAmount(e.monthly_spend_limit),
                  })
                : translateUi("Up to {{count}} lines", { count: e.max_lines })
            }
          />
        )}
        {e && !e.revoked_at && !accepted && (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => setTerms(true)}>
              {translateUi("Review and accept the current terms")}
            </button>
          </GroupedRow>
        )}
        {e && !e.revoked_at && (
          <GroupedRow asChild interactive>
            <button type="button" disabled={!canClaim} onClick={() => setChoosing(true)}>
              {translateUi("Get a NyxID number")}
            </button>
          </GroupedRow>
        )}
      </GroupedList>
      {claimed.map((line) => (
        <LineDetail key={line.id} line={line} eligibility={e ?? null} />
      ))}
      {lines.hasNextPage && (
        <GroupedList>
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={lines.isFetchingNextPage}
              onClick={() => void lines.fetchNextPage()}
            >
              {translateUi("Load more numbers")}
            </button>
          </GroupedRow>
        </GroupedList>
      )}
      {e && (
        <TermsDialog open={terms} eligibility={e} onClose={() => setTerms(false)} />
      )}
      <AvailableNumbersDialog
        open={choosing}
        lines={available}
        onClose={() => setChoosing(false)}
        onQuoted={(line, quote) => {
          setChoosing(false);
          setClaiming({ line, quote });
        }}
      />
      {e && claiming && (
        <ClaimDialog
          key={claiming.quote.id}
          line={claiming.line}
          quote={claiming.quote}
          eligibility={e}
          onClose={() => setClaiming(undefined)}
        />
      )}
    </section>
  );
}

function TermsDialog({
  open,
  eligibility: e,
  onClose,
}: {
  readonly open: boolean;
  readonly eligibility: ManagedEligibility;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const [error, setError] = useState<string>();
  const eua = e.required_terms.elevenlabs_eua;
  const form = useAppForm<ManagedTermsForm>({
    resolver: zodResolver(
      managedTermsFormSchema.refine((v) => !eua || v.accept_eua, {
        path: ["accept_eua"],
        message: "Accept the ElevenLabs End User Agreement",
      }),
    ),
    mode: "onChange",
    defaultValues: { accept_terms: false, accept_eua: false },
  });
  async function save() {
    setError(undefined);
    try {
      await actions.acceptTerms.mutateAsync({
        revision: e.revision,
        terms: e.required_terms,
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not record your acceptance.")));
    }
  }
  return (
    <GroupedDialog open={open} onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Managed line terms")}</DialogTitle>
          <DialogDescription>{translateUi("Accepting records these exact versions for your business. New lines and renewals require the current versions.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="space-y-6">
            <GroupedList title={translateUi("Terms")}>
              <ConsentField
                control={form.control}
                name="accept_terms"
                label={translateUi("I accept the NyxID managed line terms, version {{version}}.", {
                  version: e.required_terms.managed_line,
                })}
              />
              {eua && (
                <ConsentField
                  control={form.control}
                  name="accept_eua"
                  label={translateUi("I accept the ElevenLabs End User Agreement, version {{version}}, for premium voices.", {
                    version: eua,
                  })}
                />
              )}
              <GroupedFooter>{error && <ErrorBanner message={error} />}</GroupedFooter>
            </GroupedList>
            <GroupedDialogActions
              intent="create"
              label={translateUi("managed line terms")}
              dirty={form.formState.isDirty}
              pending={actions.acceptTerms.isPending}
              disabled={!form.formState.isValid}
              save={form.handleSubmit(save)}
              discard={() => form.reset()}
              saveLabel={translateUi("Accept terms")}
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function ConsentField<T extends FieldValues>({
  control,
  name,
  label,
}: {
  readonly control: Control<T>;
  readonly name: FieldPath<T>;
  readonly label: string;
}) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <Label className="flex items-start gap-2">
            <Checkbox
              checked={field.value === true}
              onCheckedChange={(value) => field.onChange(value === true)}
            />
            <span>{label}</span>
          </Label>
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

function AvailableNumbersDialog({
  open,
  lines,
  onClose,
  onQuoted,
}: {
  readonly open: boolean;
  readonly lines: readonly ManagedLine[];
  readonly onClose: () => void;
  readonly onQuoted: (line: ManagedLine, quote: ManagedQuote) => void;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const [error, setError] = useState<string>();
  async function price(line: ManagedLine) {
    setError(undefined);
    try {
      onQuoted(line, await actions.quote.mutateAsync({ lineId: line.id, kind: { kind: "claim" } }));
    } catch (cause) {
      setError(failure(cause, translateUi("Could not price this number.")));
    }
  }
  return (
    <GroupedDialog open={open} onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{translateUi("Get a NyxID number")}</DialogTitle>
          <DialogDescription>{translateUi("Choose a number to see its current prices. Nothing is charged until you accept a quote.")}</DialogDescription>
        </DialogHeader>
        <GroupedList
          title={translateUi("Available numbers")}
          footer={error ? <ErrorBanner message={error} /> : undefined}
        >
          {lines.length === 0 && (
            <GroupedRow title={translateUi("No numbers are available right now.")} />
          )}
          {lines.map((line) => (
            <GroupedRow
              key={line.id}
              title={line.phone_number}
              description={[
                line.country,
                line.voice ? translateUi("Voice") : null,
                line.sms ? translateUi("SMS") : null,
              ]
                .filter(Boolean)
                .join(" · ")}
              accessory={
                <button
                  type="button"
                  className="text-foreground underline"
                  disabled={actions.quote.isPending}
                  onClick={() => void price(line)}
                >
                  {translateUi("See prices")}
                </button>
              }
            />
          ))}
        </GroupedList>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function QuoteRows({ quote }: { readonly quote: ManagedQuote }) {
  useUiTranslation();
  return (
    <GroupedList
      title={translateUi("Quote")}
      footer={
        <p>{translateUi("Prices are valid until {{time}}. Usage is charged at the rate current when each call or message is admitted.", {
          time: when(quote.accept_before),
        })}</p>
      }
    >
      <GroupedRow
        title={translateUi("Line rental")}
        description={translateUi("{{amount}} per {{days}} days ({{rate}} per day), prepaid and not refundable", {
          amount: formatCreditAmount(periodRental(quote)),
          days: quote.period_days,
          rate: formatCreditAmount(quote.rental.credits_per_unit),
        })}
      />
      {quote.usage.map((rate) => (
        <GroupedRow key={`${rate.purpose}-${rate.revision}`} title={rateText(rate)} />
      ))}
    </GroupedList>
  );
}

function ClaimDialog({
  line,
  quote,
  eligibility: e,
  onClose,
}: {
  readonly line: ManagedLine;
  readonly quote: ManagedQuote;
  readonly eligibility: ManagedEligibility;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  const agents = useNyxBotAgents();
  const actions = useManagedLineActions();
  // One request identity per dialog: a retry after a lost response replays the
  // original claim instead of creating another.
  const [requestId] = useState(newRequestId);
  const [error, setError] = useState<string>();
  const specialists =
    agents.data?.agents.filter(
      (a) =>
        a.kind === "specialist" &&
        a.status !== "destroyed" &&
        a.owner_kind !== "org" &&
        (!a.owner_id || a.owner_id === actor),
    ) ?? [];
  const standard = quote.standard_voice_options.filter((o) => !isPremiumVoice(o));
  const premium = e.premium_allowed
    ? quote.standard_voice_options.filter(isPremiumVoice)
    : [];
  const smsReady = line.sms && e.sms_registration_status === "approved";
  const form = useAppForm<ManagedClaimForm>({
    resolver: zodResolver(managedClaimFormSchema),
    mode: "onChange",
    defaultValues: {
      agent_id: "",
      voice_tier: "standard",
      voice_option: standard[0] ? voiceKey(standard[0]) : "",
      enable_sms: false,
      caller_confirmations: false,
      // Recurring charges need an affirmative choice; never pre-select it.
      auto_renew: false,
      accept_price: false,
      accept_terms: false,
      accept_eua: false,
    },
  });
  const tier = form.watch("voice_tier");
  const options = tier === "premium" ? premium : standard;
  const sms = form.watch("enable_sms");
  async function save(v: ManagedClaimForm) {
    setError(undefined);
    const option = options.find((o) => voiceKey(o) === v.voice_option) ?? null;
    if (line.voice && !option) {
      setError(translateUi("Choose a voice."));
      return;
    }
    try {
      await actions.claim.mutateAsync({
        request_id: requestId,
        quote_id: quote.id,
        line_id: line.id,
        agent_id: v.agent_id,
        voice_tier: v.voice_tier,
        standard_voice: line.voice ? option : null,
        enable_sms: v.enable_sms,
        caller_confirmations: v.caller_confirmations,
        auto_renew: v.auto_renew,
        accepted_terms_versions: quote.terms,
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not get this number.")));
    }
  }
  return (
    <GroupedDialog open onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{line.phone_number}</DialogTitle>
          <DialogDescription>{translateUi("Review the prices, choose the specialist that answers this number, and confirm. The first 30-day rental is charged when you confirm.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="space-y-6">
            <QuoteRows quote={quote} />
            <GroupedList title={translateUi("Line settings")}>
              <FormField
                control={form.control}
                name="agent_id"
                render={({ field }) => (
                  <FormItem inline>
                    <FormLabel>{translateUi("Specialist")}</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder={translateUi("Choose your specialist")} />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {specialists.map((a) => (
                          <SelectItem key={a.id} value={a.id}>
                            {a.display_name ?? a.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {line.voice && (
                <FormField
                  control={form.control}
                  name="voice_tier"
                  render={({ field }) => (
                    <FormItem inline>
                      <FormLabel>{translateUi("Voice tier")}</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={(value) => {
                          field.onChange(value);
                          const next = value === "premium" ? premium : standard;
                          form.setValue("voice_option", next[0] ? voiceKey(next[0]) : "");
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="standard">{translateUi("Standard")}</SelectItem>
                          {premium.length > 0 && (
                            <SelectItem value="premium">{translateUi("Premium (ElevenLabs)")}</SelectItem>
                          )}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}
              {line.voice && (
                <FormField
                  control={form.control}
                  name="voice_option"
                  render={({ field }) => (
                    <FormItem inline>
                      <FormLabel>{translateUi("Voice and language")}</FormLabel>
                      <Select value={field.value} onValueChange={field.onChange}>
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {options.map((o) => (
                            <SelectItem key={voiceKey(o)} value={voiceKey(o)}>
                              {`${o.voice} · ${o.language}`}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}
              {line.sms && (
                <SwitchField
                  control={form.control}
                  name="enable_sms"
                  label={translateUi("Answer inbound SMS")}
                  disabled={!smsReady}
                  onChange={(on) => {
                    if (!on) form.setValue("caller_confirmations", false);
                  }}
                />
              )}
              {line.sms && !smsReady && (
                <GroupedFooter>
                  <p>{translateUi("SMS becomes available after NyxID approves your SMS sender registration.")}</p>
                </GroupedFooter>
              )}
              {line.voice && line.sms && (
                <SwitchField
                  control={form.control}
                  name="caller_confirmations"
                  label={translateUi("Text booking confirmations to callers")}
                  disabled={!sms}
                />
              )}
              <SwitchField
                control={form.control}
                name="auto_renew"
                label={translateUi("Renew automatically at this price")}
              />
              <GroupedFooter>
                <p>{translateUi("Auto-renew charges the next 30 days before coverage ends, only while the price and terms are unchanged. Otherwise NyxID asks you to accept a new quote.")}</p>
              </GroupedFooter>
            </GroupedList>
            <GroupedList title={translateUi("Consent")}>
              <ConsentField
                control={form.control}
                name="accept_price"
                label={translateUi("I accept these prices. The first 30-day rental is charged now; usage is charged as it occurs.")}
              />
              <ConsentField
                control={form.control}
                name="accept_terms"
                label={translateUi("I accept the NyxID managed line terms, version {{version}}.", {
                  version: quote.terms.managed_line,
                })}
              />
              {tier === "premium" && quote.terms.elevenlabs_eua && (
                <ConsentField
                  control={form.control}
                  name="accept_eua"
                  label={translateUi("I accept the ElevenLabs End User Agreement, version {{version}}, for premium voices.", {
                    version: quote.terms.elevenlabs_eua,
                  })}
                />
              )}
              <GroupedFooter>{error && <ErrorBanner message={error} />}</GroupedFooter>
            </GroupedList>
            <GroupedDialogActions
              intent="create"
              label={translateUi("NyxID number")}
              dirty={form.formState.isDirty}
              pending={actions.claim.isPending}
              disabled={!form.formState.isValid}
              save={form.handleSubmit(save)}
              discard={() => {
                form.reset();
                setError(undefined);
              }}
              saveLabel={
                actions.claim.isPending
                  ? translateUi("Getting number…")
                  : translateUi("Pay and get this number")
              }
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function SwitchField<T extends FieldValues>({
  control,
  name,
  label,
  disabled,
  onChange,
}: {
  readonly control: Control<T>;
  readonly name: FieldPath<T>;
  readonly label: string;
  readonly disabled?: boolean;
  readonly onChange?: (on: boolean) => void;
}) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem inline>
          <FormLabel>{label}</FormLabel>
          <FormControl>
            <Switch
              checked={field.value === true}
              disabled={disabled}
              onCheckedChange={(on) => {
                field.onChange(on);
                onChange?.(on);
              }}
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

function LineDetail({
  line,
  eligibility: e,
}: {
  readonly line: ManagedLine;
  readonly eligibility: ManagedEligibility | null;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const agents = useNyxBotAgents();
  const panel = useAccountPanel();
  const [error, setError] = useState<string>();
  const [coverage, setCoverage] = useState<ManagedQuote>();
  const [voice, setVoice] = useState(false);
  const [release, setRelease] = useState(false);
  const c = line.claim!;
  const agent = agents.data?.agents.find((a) => a.id === c.agent_id);
  const unpaid = c.state === "suspended" && c.suspension === "unpaid";
  async function quote(kind: "renewal" | "resume") {
    setError(undefined);
    try {
      setCoverage(
        await actions.quote.mutateAsync({
          lineId: line.id,
          kind:
            kind === "renewal"
              ? {
                  kind: "renewal",
                  claim_id: c.id,
                  generation: c.generation,
                  period_index: (c.period_index ?? 0) + 1,
                }
              : { kind: "resume", claim_id: c.id, generation: c.generation },
        }),
      );
    } catch (cause) {
      setError(failure(cause, translateUi("Could not get a quote for this line.")));
    }
  }
  async function toggle(on: boolean) {
    setError(undefined);
    try {
      await actions.confirmations.mutateAsync({ line, enabled: on });
    } catch (cause) {
      setError(failure(cause, translateUi("Could not change caller confirmations.")));
    }
  }
  return (
    <>
      <GroupedList
        title={line.phone_number}
        footer={
          <>
            {c.state === "releasing" && (
              <p>{translateUi("Release finishes after every call and message on this line is accounted for. Calls are no longer answered.")}</p>
            )}
            {unpaid && (
              <p>{translateUi("Paid coverage ended, so calls are rejected without charge. Resume within seven days of suspension to keep this number; resume starts a new 30-day period from now.")}</p>
            )}
            {error && <ErrorBanner message={error} />}
          </>
        }
      >
        <GroupedRow
          title={translateUi("Status")}
          accessory={stateLabel(c.state, c.suspension)}
        />
        <GroupedRow
          title={translateUi("Paid coverage")}
          description={translateUi("{{from}} to {{until}} (the end time is not included)", {
            from: when(c.covered_from),
            until: when(c.covered_until),
          })}
        />
        <GroupedRow
          title={translateUi("Specialist")}
          accessory={agent ? (agent.display_name ?? agent.name) : c.agent_id ?? "—"}
        />
        {line.voice && (
          <GroupedRow
            title={translateUi("Voice")}
            description={
              c.standard_voice
                ? `${c.voice_tier === "premium" ? translateUi("Premium") : translateUi("Standard")} · ${c.standard_voice.voice} · ${c.standard_voice.language}`
                : undefined
            }
            accessory={
              c.state === "claimed" || c.state === "suspended" ? (
                <button type="button" className="text-foreground underline" onClick={() => setVoice(true)}>
                  {translateUi("Change")}
                </button>
              ) : undefined
            }
          />
        )}
        <GroupedRow
          title={translateUi("SMS replies")}
          accessory={c.sms_enabled ? translateUi("On") : translateUi("Off")}
        />
        {line.voice && c.sms_enabled && (
          <GroupedRow
            title={translateUi("Caller confirmations by SMS")}
            description={translateUi("Your specialist may text the current caller at most twice per call. Each text is charged as SMS.")}
            accessory={
              <Switch
                aria-label={translateUi("Caller confirmations by SMS")}
                checked={c.caller_confirmations === true}
                disabled={actions.confirmations.isPending || c.state !== "claimed"}
                onCheckedChange={(on) => void toggle(on)}
              />
            }
          />
        )}
        <GroupedRow
          title={translateUi("Automatic renewal")}
          accessory={c.auto_renew ? translateUi("On") : translateUi("Off")}
        />
        {c.state === "claimed" && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={actions.quote.isPending}
              onClick={() =>
                c.renewal_quote ? setCoverage(c.renewal_quote) : void quote("renewal")
              }
            >
              {c.renewal_quote
                ? translateUi("Review renewal quote")
                : translateUi("Pay for the next 30 days")}
            </button>
          </GroupedRow>
        )}
        {unpaid && (
          <GroupedRow asChild interactive>
            <button type="button" disabled={actions.quote.isPending} onClick={() => void quote("resume")}>
              {translateUi("Resume this number")}
            </button>
          </GroupedRow>
        )}
        <GroupedRow asChild interactive>
          <button type="button" onClick={() => void panel.open("billing", { panelTab: "usage" })}>
            {translateUi("View usage in Billing & Usage")}
          </button>
        </GroupedRow>
        {(c.state === "claimed" || c.state === "suspended") && (
          <GroupedRow asChild interactive>
            <button type="button" className="text-destructive" onClick={() => setRelease(true)}>
              {translateUi("Release this number")}
            </button>
          </GroupedRow>
        )}
      </GroupedList>
      {coverage && e && (
        <CoverageDialog
          key={coverage.id}
          line={line}
          quote={coverage}
          onClose={() => setCoverage(undefined)}
        />
      )}
      {voice && <VoiceDialog line={line} onClose={() => setVoice(false)} />}
      {release && <ReleaseDialog line={line} onClose={() => setRelease(false)} />}
    </>
  );
}

function CoverageDialog({
  line,
  quote,
  onClose,
}: {
  readonly line: ManagedLine;
  readonly quote: ManagedQuote;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const [requestId] = useState(newRequestId);
  const [error, setError] = useState<string>();
  const resume = quote.kind.kind === "resume";
  const form = useAppForm<ManagedCoverageForm>({
    resolver: zodResolver(managedCoverageFormSchema),
    mode: "onChange",
    defaultValues: {
      auto_renew: line.claim?.auto_renew ?? true,
      accept_price: false,
      accept_terms: false,
    },
  });
  async function save(v: ManagedCoverageForm) {
    setError(undefined);
    try {
      await actions.coverage.mutateAsync({
        request_id: requestId,
        quote_id: quote.id,
        auto_renew: v.auto_renew,
        accepted_terms_versions: quote.terms,
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not fund this period.")));
    }
  }
  return (
    <GroupedDialog open onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {resume ? translateUi("Resume {{number}}", { number: line.phone_number }) : translateUi("Renew {{number}}", { number: line.phone_number })}
          </DialogTitle>
          <DialogDescription>
            {resume
              ? translateUi("Resuming starts a new 30-day period now. Suspended time is never charged.")
              : translateUi("The next 30 days start when the current paid period ends.")}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="space-y-6">
            <QuoteRows quote={quote} />
            <GroupedList title={translateUi("Consent")}>
              <SwitchField
                control={form.control}
                name="auto_renew"
                label={translateUi("Renew automatically at this price")}
              />
              <ConsentField
                control={form.control}
                name="accept_price"
                label={translateUi("I accept this price for the next 30 days. It is charged now and not refunded.")}
              />
              <ConsentField
                control={form.control}
                name="accept_terms"
                label={translateUi("I accept the NyxID managed line terms, version {{version}}.", {
                  version: quote.terms.managed_line,
                })}
              />
              <GroupedFooter>{error && <ErrorBanner message={error} />}</GroupedFooter>
            </GroupedList>
            <GroupedDialogActions
              intent="create"
              label={translateUi("line coverage")}
              dirty={form.formState.isDirty}
              pending={actions.coverage.isPending}
              disabled={!form.formState.isValid}
              save={form.handleSubmit(save)}
              discard={() => form.reset()}
              saveLabel={resume ? translateUi("Pay and resume") : translateUi("Pay and renew")}
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function VoiceDialog({
  line,
  onClose,
}: {
  readonly line: ManagedLine;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const current = line.claim?.standard_voice;
  const [choice, setChoice] = useState(
    current ? voiceKey(current) : line.standard_voice_options[0] ? voiceKey(line.standard_voice_options[0]) : "",
  );
  const [error, setError] = useState<string>();
  async function save() {
    const option = line.standard_voice_options.find((o) => voiceKey(o) === choice);
    if (!option) return;
    setError(undefined);
    try {
      await actions.setVoice.mutateAsync({ line, option });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not change the voice. End any live call and try again.")));
    }
  }
  return (
    <GroupedDialog open onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Change voice")}</DialogTitle>
          <DialogDescription>{translateUi("Standard voices are included in the call price. Changes apply to the next call and are refused while a call is live.")}</DialogDescription>
        </DialogHeader>
        <GroupedList footer={error ? <ErrorBanner message={error} /> : undefined}>
          <GroupedRow>
            <Label className="min-w-0 flex-1 text-foreground" htmlFor="managed-line-voice">
              {translateUi("Voice and language")}
            </Label>
            <Select value={choice} onValueChange={setChoice}>
              <SelectTrigger id="managed-line-voice">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {line.standard_voice_options.map((o) => (
                  <SelectItem key={voiceKey(o)} value={voiceKey(o)}>
                    {`${o.voice} · ${o.language}`}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </GroupedRow>
        </GroupedList>
        <GroupedDialogActions
          intent="create"
          label={translateUi("voice")}
          dirty={!current || choice !== voiceKey(current)}
          pending={actions.setVoice.isPending}
          disabled={!choice || (current != null && choice === voiceKey(current))}
          save={save}
          discard={() => setChoice(current ? voiceKey(current) : "")}
          saveLabel={translateUi("Change voice")}
        />
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function ReleaseDialog({
  line,
  onClose,
}: {
  readonly line: ManagedLine;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useManagedLineActions();
  const [requestId] = useState(newRequestId);
  const [error, setError] = useState<string>();
  async function save() {
    const claim = line.claim;
    if (!claim) return;
    setError(undefined);
    try {
      await actions.release.mutateAsync({
        request_id: requestId,
        line_id: line.id,
        claim_id: claim.id,
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not release this number. Retry to continue the same release.")));
    }
  }
  return (
    <GroupedDialog open onOpenChange={(next) => !next && onClose()}>
      <GroupedDialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Release {{number}}?", { number: line.phone_number })}</DialogTitle>
          <DialogDescription>{translateUi("Calls and SMS stop now and the number returns to NyxID after a cooldown. The current paid period is not refunded and you cannot undo a release.")}</DialogDescription>
        </DialogHeader>
        {error && <ErrorBanner message={error} />}
        <GroupedDialogActions
          intent="create"
          label={translateUi("number release")}
          dirty
          pending={actions.release.isPending}
          save={save}
          discard={onClose}
          saveVariant="destructive"
          saveLabel={translateUi("Release number")}
        />
      </GroupedDialogContent>
    </GroupedDialog>
  );
}
