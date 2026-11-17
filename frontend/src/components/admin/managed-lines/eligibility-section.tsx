import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useAdminUsers } from "@/hooks/use-admin";
import {
  useAdminManagedLineActions,
  useAdminManagedLines,
} from "@/hooks/use-admin-managed-lines";
import {
  eligibilityFormSchema,
  type EligibilityForm,
} from "@/schemas/admin-managed-lines";
import type { ManagedEligibility } from "@/schemas/managed-lines";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
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
  TextField,
} from "./fields";
import { failure, when } from "./format";

function defaults(row?: ManagedEligibility): EligibilityForm {
  return {
    owner_id: row?.owner_id ?? "",
    business_legal_name: row?.business_legal_name ?? "",
    registration_id: row?.registration_id ?? "",
    jurisdiction: row?.jurisdiction ?? "",
    allowed_use_case: row?.allowed_use_case ?? "",
    sms_sender_path: row?.sms_sender_path ?? "",
    sms_registration_status: row?.sms_registration_status ?? "",
    managed_line_terms: row?.required_terms.managed_line ?? "",
    elevenlabs_eua: row?.required_terms.elevenlabs_eua ?? "",
    premium_allowed: row?.premium_allowed ?? false,
    max_lines: row?.max_lines ?? 1,
    monthly_spend_limit: row?.monthly_spend_limit ?? "",
  };
}

export function EligibilitySection() {
  useUiTranslation();
  const { eligibility } = useAdminManagedLines();
  const actions = useAdminManagedLineActions();
  const [editing, setEditing] = useState<{ row?: ManagedEligibility }>();
  const [confirm, setConfirm] = useState<{ kind: "apply" | "revoke"; row: ManagedEligibility }>();
  const [error, setError] = useState<string>();
  async function decide() {
    if (!confirm) return;
    setError(undefined);
    try {
      const input = { ownerId: confirm.row.owner_id, revision: confirm.row.revision };
      if (confirm.kind === "apply") await actions.applyEligibility.mutateAsync(input);
      else await actions.revokeEligibility.mutateAsync(input);
      setConfirm(undefined);
    } catch (cause) {
      setError(failure(cause, translateUi("Eligibility change failed.")));
    }
  }
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="font-medium">{translateUi("Merchant eligibility")}</h3>
        <Button variant="outline" onClick={() => setEditing({})}>
          {translateUi("Grant eligibility")}
        </Button>
      </div>
      <p className="text-muted-foreground">{translateUi("Approve a person’s business before they can claim a line. Merchants accept terms themselves and cannot self-approve. Changes that need no new consent reach existing claims only after Apply.")}</p>
      {eligibility.error && (
        <ErrorBanner
          message={translateUi("Could not load merchant eligibility.")}
          onRetry={() => void eligibility.refetch()}
        />
      )}
      {error && <ErrorBanner message={error} />}
      {eligibility.data?.map((row) => {
        const accepted =
          row.accepted_terms?.managed_line === row.required_terms.managed_line &&
          (row.accepted_terms?.elevenlabs_eua ?? null) === row.required_terms.elevenlabs_eua;
        return (
          <article key={row.id} className="space-y-2 rounded-lg border border-border p-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <p>
                {row.business_legal_name} · {row.jurisdiction} · {row.owner_id}
              </p>
              <div className="flex flex-wrap gap-2">
                {row.revoked_at ? (
                  <Badge variant="destructive">{translateUi("Revoked")}</Badge>
                ) : accepted ? (
                  <Badge variant="success">{translateUi("Terms accepted")}</Badge>
                ) : (
                  <Badge variant="warning">{translateUi("Awaiting merchant acceptance")}</Badge>
                )}
              </div>
            </div>
            <p className="text-muted-foreground">
              {translateUi("Revision {{revision}} · terms {{terms}} · up to {{count}} lines · premium {{premium}}", {
                revision: row.revision,
                terms: row.required_terms.managed_line,
                count: row.max_lines,
                premium: row.premium_allowed ? translateUi("allowed") : translateUi("not allowed"),
              })}
            </p>
            <p className="text-muted-foreground">
              {translateUi("Approved {{time}}", { time: when(row.approved_at) })}
            </p>
            {!row.revoked_at && (
              <div className="flex flex-wrap gap-2">
                <Button variant="outline" onClick={() => setEditing({ row })}>
                  {translateUi("Update")}
                </Button>
                <Button
                  variant="outline"
                  disabled={!accepted}
                  onClick={() => setConfirm({ kind: "apply", row })}
                >
                  {translateUi("Apply revision to claims")}
                </Button>
                <Button variant="outline" onClick={() => setConfirm({ kind: "revoke", row })}>
                  {translateUi("Revoke")}
                </Button>
              </div>
            )}
          </article>
        );
      })}
      {eligibility.hasNextPage && (
        <Button
          variant="outline"
          disabled={eligibility.isFetchingNextPage}
          onClick={() => void eligibility.fetchNextPage()}
        >
          {translateUi("Load more merchants")}
        </Button>
      )}
      {editing && (
        <EligibilityDialog row={editing.row} onClose={() => setEditing(undefined)} />
      )}
      <Dialog open={!!confirm} onOpenChange={(open) => !open && setConfirm(undefined)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirm?.kind === "revoke"
                ? translateUi("Revoke eligibility?")
                : translateUi("Apply this revision to claims?")}
            </DialogTitle>
            <DialogDescription>
              {confirm?.kind === "revoke"
                ? translateUi("Revocation suspends every claim of this merchant, disables its bindings and stops new calls and messages. Existing effects still settle.")
                : translateUi("Claims adopt this eligibility revision. Use this only for changes the merchant has already accepted.")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirm(undefined)}>
              {translateUi("Cancel")}
            </Button>
            <Button
              variant={confirm?.kind === "revoke" ? "destructive" : "primary"}
              disabled={actions.applyEligibility.isPending || actions.revokeEligibility.isPending}
              onClick={() => void decide()}
            >
              {confirm?.kind === "revoke" ? translateUi("Revoke") : translateUi("Apply")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function EligibilityDialog({
  row,
  onClose,
}: {
  readonly row?: ManagedEligibility;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const actions = useAdminManagedLineActions();
  const [search, setSearch] = useState("");
  const users = useAdminUsers(1, 20, search, "person", { enabled: !row });
  const [error, setError] = useState<string>();
  const form = useAppForm<EligibilityForm>({
    resolver: zodResolver(eligibilityFormSchema),
    mode: "onChange",
    defaultValues: defaults(row),
  });
  async function save(v: EligibilityForm) {
    setError(undefined);
    try {
      await actions.saveEligibility.mutateAsync({
        ownerId: v.owner_id,
        body: {
          business_legal_name: v.business_legal_name,
          registration_id: v.registration_id,
          jurisdiction: v.jurisdiction,
          allowed_use_case: v.allowed_use_case,
          sms_sender_path: v.sms_sender_path,
          sms_registration_status: v.sms_registration_status,
          required_terms: {
            managed_line: v.managed_line_terms,
            elevenlabs_eua: v.elevenlabs_eua || null,
          },
          premium_allowed: v.premium_allowed,
          max_lines: v.max_lines,
          monthly_spend_limit: v.monthly_spend_limit || null,
          expected_revision: row?.revision ?? null,
        },
      });
      onClose();
    } catch (cause) {
      setError(failure(cause, translateUi("Could not save eligibility.")));
    }
  }
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {row ? translateUi("Update eligibility") : translateUi("Grant eligibility")}
          </DialogTitle>
          <DialogDescription>{translateUi("Changing a terms version requires the merchant to accept it before new admissions continue. Audit records metadata only.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
            {!row && (
              <>
                <Input
                  aria-label={translateUi("Search people by email")}
                  placeholder={translateUi("Search people by email")}
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                />
                <ChoiceField
                  control={form.control}
                  name="owner_id"
                  label={translateUi("Person")}
                  placeholder={translateUi("Choose a person")}
                  choices={(users.data?.users ?? []).map((u) => ({
                    value: u.id,
                    label: u.email,
                  }))}
                />
              </>
            )}
            <TextField control={form.control} name="business_legal_name" label={translateUi("Business legal name")} />
            <TextField control={form.control} name="registration_id" label={translateUi("Business registration ID")} />
            <TextField control={form.control} name="jurisdiction" label={translateUi("Jurisdiction (ISO country)")} />
            <TextField control={form.control} name="allowed_use_case" label={translateUi("Allowed use case")} />
            <TextField control={form.control} name="sms_sender_path" label={translateUi("SMS sender path")} />
            <TextField
              control={form.control}
              name="sms_registration_status"
              label={translateUi("SMS registration status")}
              help={translateUi("Merchants can enable SMS only when this is approved.")}
            />
            <TextField control={form.control} name="managed_line_terms" label={translateUi("Managed line terms version")} />
            <CheckField control={form.control} name="premium_allowed" label={translateUi("Allow premium ElevenLabs voices")} />
            <TextField
              control={form.control}
              name="elevenlabs_eua"
              label={translateUi("ElevenLabs End User Agreement version")}
              help={translateUi("Required when premium voices are allowed; it must match the account’s OEM terms version.")}
            />
            <NumberField control={form.control} name="max_lines" label={translateUi("Maximum lines")} />
            <TextField
              control={form.control}
              name="monthly_spend_limit"
              label={translateUi("Monthly phone spend limit (credits)")}
              help={translateUi("Pre-tax rental, calls, premium voice and SMS by UTC month, including benefit-funded usage. Leave empty for no limit.")}
            />
            {error && <ErrorBanner message={error} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                {translateUi("Cancel")}
              </Button>
              <Button
                type="submit"
                disabled={!form.formState.isValid || actions.saveEligibility.isPending}
              >
                {translateUi("Save eligibility")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
