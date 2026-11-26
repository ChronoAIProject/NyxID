import { useId, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { ChevronRight } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { DialogBody, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { useAppForm } from "@/components/ui/form";
import { GroupedDialog, GroupedDialogCancel, GroupedDialogContent } from "@/components/ui/grouped-dialog";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import { GroupedInput, GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { Label } from "@/components/ui/label";
import { useOrgServiceApprovers } from "@/hooks/use-approvals";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { ApiError } from "@/lib/api-client";
import { MAX_SERVICE_APPROVERS, serviceApproversDraft, type ApproverIneligibility, type ServiceApprovers, type ServiceApproversDraft } from "@/schemas/service-approvers";

function mode(ids: string[] | null) {
  return ids === null ? translateUi("All eligible Admins") : translateUi("Selected Admins");
}

function ineligibilityLabel(reason?: ApproverIneligibility) {
  switch (reason) {
    case "not_admin": return translateUi("Admin role required");
    case "not_member": return translateUi("no current organization membership");
    case "out_of_scope": return translateUi("outside this Admin’s service scope");
    case "inactive": return translateUi("inactive account");
    default: return translateUi("reason unknown");
  }
}

function readOnlyMessage(reason: ServiceApprovers["authoring_unavailable_reason"]) {
  switch (reason) {
    case "flag_off": return translateUi("Selecting approvers isn't enabled for this organization yet.");
    case "readiness_missing": return translateUi("The servers aren't ready for approver changes yet.");
    default: return translateUi("Editing is unavailable");
  }
}

export function OrgServiceApprovers({ orgId, serviceId, serviceName }: {
  orgId: string; serviceId: string; serviceName: string;
}) {
  const { t } = useUiTranslation();
  const { policy, replace } = useOrgServiceApprovers(orgId, serviceId);
  const [open, setOpen] = useState(false);
  const { data, error } = policy;
  // Older replicas lack this resource. The parent policy remains usable.
  if (error instanceof ApiError && error.status === 404) return null;
  if (error instanceof ApiError && error.status === 403) {
    return <p className="mt-3 type-secondary">{t("You don't manage this service's approvals.")}</p>;
  }
  if (!data) return error
    ? <ErrorBanner message={t("Could not load service approvers.")} onRetry={() => void policy.refetch()} />
    : <p role="status" className="mt-3 type-secondary">{t("Loading service approvers…")}</p>;

  return <GroupedDialog open={open} onOpenChange={setOpen}>
    <DialogTrigger asChild>
      <Button type="button" variant="ghost" className="mt-3 w-full justify-between" aria-label={t("Approvers for {{service}}", { service: serviceName })}>
        {t("Service approvers")}
        <span className="flex items-center gap-1 text-muted-foreground">
          {mode(data.approver_user_ids)} <ChevronRight className="size-4" aria-hidden />
        </span>
      </Button>
    </DialogTrigger>
    {open && <GroupedDialogContent scrollMode="body">
      <DialogHeader>
        <DialogTitle>{t("Service approvers")}</DialogTitle>
        <DialogDescription>{serviceName}</DialogDescription>
      </DialogHeader>
      <ApproverSettings saved={data} replace={replace} policy={policy} close={() => setOpen(false)} />
    </GroupedDialogContent>}
  </GroupedDialog>;
}

function ApproverSettings({ saved, policy, replace, close }: ReturnType<typeof useOrgServiceApprovers> & {
  saved: ServiceApprovers; close: () => void;
}) {
  const { t } = useUiTranslation();
  const group = useId();
  const [base, setBase] = useState(saved);
  const [conflict, setConflict] = useState(false);
  const [search, setSearch] = useState("");
  const form = useAppForm<ServiceApproversDraft>({
    defaultValues: { approver_user_ids: base.approver_user_ids },
    resolver: zodResolver(serviceApproversDraft),
  });
  const readOnly = !saved.authoring_available;
  const watched = form.watch("approver_user_ids");
  const ids = readOnly ? saved.approver_user_ids : watched;
  const candidates = saved.eligible_candidates;
  const names = new Map(candidates.map((p) => [p.user_id, p.display_name || p.user_id]));
  const unavailablePeople = new Map(saved.unavailable?.map((person) => [person.user_id, person]));
  function unavailableLabel(id: string) {
    const person = unavailablePeople.get(id);
    return t("{{name}} (no longer eligible: {{reason}})", {
      name: person?.display_name ?? `${id.slice(0, 8)}…${id.slice(-4)}`,
      reason: ineligibilityLabel(person?.reason),
    });
  }
  const unavailable = ids?.filter((id) => !names.has(id)) ?? [];
  const searchable = !readOnly && candidates.length > 8;
  const term = searchable ? search.trim().toLowerCase() : "";
  const visible = candidates.filter((p) =>
    (!readOnly || ids?.includes(p.user_id)) &&
    [p.user_id, p.display_name].some((value) => value?.toLowerCase().includes(term)),
  );
  const stale = conflict || base.revision !== saved.revision || base.config_id !== saved.config_id;
  const original = base.approver_user_ids;
  const dirty = ids === null || original === null ? ids !== original
    : ids.length !== original.length || ids.some((id) => !original.includes(id));
  const invalid = ids !== null && (ids.length === 0 || ids.length > MAX_SERVICE_APPROVERS || unavailable.length > 0);
  const disabled = readOnly || stale || invalid || policy.isError || replace.isPending;

  function reset(next: ServiceApprovers) {
    form.reset({ approver_user_ids: next.approver_user_ids });
    setBase(next);
    setConflict(false);
    setSearch("");
    replace.reset();
  }
  async function reload() {
    const result = await policy.refetch();
    if (result.isSuccess) reset(result.data);
  }
  const save = form.handleSubmit(async (draft) => {
    if (!dirty || disabled) return;
    try {
      await replace.mutateAsync({ ...draft, config_id: base.config_id, expected_revision: base.revision });
      close();
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 409) setConflict(true);
      else if (cause instanceof ApiError && [400, 403, 404].includes(cause.status)) void policy.refetch();
    }
  });
  function describe(value: string[] | null) {
    return value === null ? mode(null) : value.map((id) => names.get(id) ?? unavailableLabel(id)).join(", ") || t("Nobody");
  }

  return <>
    <DialogBody className="space-y-4 py-4">
      {readOnly && <p className="type-secondary">{readOnlyMessage(saved.authoring_unavailable_reason)}</p>}
      <fieldset disabled={readOnly || replace.isPending} className="space-y-4">
        <Label asChild><legend className="sr-only">{t("Service approvers")}</legend></Label>
        <GroupedList>
          {(["all", "selected"] as const).map((choice) => <GroupedRow asChild key={choice}>
            <Label><input type="radio" name={group} checked={(ids === null) === (choice === "all")}
              onChange={() => form.setValue("approver_user_ids", choice === "all" ? null : [])} />
              {choice === "all" ? t("All eligible Admins") : t("Selected Admins")}
            </Label>
          </GroupedRow>)}
        </GroupedList>
        {ids !== null && <GroupedList
          footer={!readOnly && t("Select 1 to 128 eligible Admins.")}>
          {searchable && <GroupedRow>
            <GroupedInput type="search" placeholder={t("Search Admins")} aria-label={t("Search Admins")}
              value={search} onChange={(event) => setSearch(event.target.value)} />
          </GroupedRow>}
          <div className="max-h-64 overflow-y-auto">
            {unavailable.map((id) => <GroupedRow asChild key={id}>
              <Label><input type="checkbox" checked onChange={() => form.setValue("approver_user_ids", ids.filter((v) => v !== id))} />
                <span className="min-w-0 break-words">{unavailableLabel(id)}</span>
              </Label>
            </GroupedRow>)}
            {visible.map(({ user_id }) => <GroupedRow asChild key={user_id}>
              <Label><input type="checkbox" checked={ids.includes(user_id)}
                disabled={ids.length >= MAX_SERVICE_APPROVERS && !ids.includes(user_id)}
                onChange={(event) => form.setValue("approver_user_ids", event.target.checked
                  ? [...ids, user_id] : ids.filter((id) => id !== user_id))} />
                <span className="min-w-0 break-words">{names.get(user_id)}</span>
              </Label>
            </GroupedRow>)}
            {term && visible.length === 0 && <GroupedRow>{t("No matches")}</GroupedRow>}
          </div>
        </GroupedList>}
      </fieldset>
      {candidates.length === 0 && <p role="status" className="type-secondary">{t("No Admins are eligible for this service.")}</p>}
      {ids?.length === 0 && <p role="status" className="type-secondary">{t("No Admins are selected. Approval requests cannot be decided with an empty list.")}</p>}
      {unavailable.length > 0 && !readOnly && <p className="type-secondary">{t("Remove unavailable Admins before saving.")}</p>}
      {stale && !readOnly && <div role="alert" className="space-y-2">
        <p className="type-secondary">{t("The saved approvers changed. Reload to replace your draft with the latest settings.")}</p>
        <Button type="button" variant="outline" disabled={policy.isFetching} onClick={() => void reload()}>{t("Reload saved")}</Button>
      </div>}
      {policy.isError && <ErrorBanner message={t("Could not load service approvers.")} onRetry={() => void policy.refetch()} />}
      {replace.isError && !conflict && !readOnly && <ErrorBanner message={t("Could not save service approvers.")} />}
    </DialogBody>
    {readOnly ? <GroupedDialogCancel className="self-end">{t("Close")}</GroupedDialogCancel> :
      <GroupedDialogActions label={t("Service approvers")} saveLabel={t("Save changes")} dirty={dirty}
        disabled={disabled} pending={replace.isPending} discard={() => reset(saved)} save={save}
        changes={() => [{ label: t("Service approvers"), before: describe(original), after: describe(ids) }]} />}
  </>;
}
