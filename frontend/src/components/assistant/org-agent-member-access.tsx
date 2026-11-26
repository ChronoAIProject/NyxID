import { Fragment, useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { GroupedDisclosure, GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useOrgAgentAccess } from "@/hooks/use-org-agent-access";
import { useOrgMembers } from "@/hooks/use-org-members";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { ApiError } from "@/lib/api-client";
import { MAX_AGENT_ACCESS_PEOPLE, orgAccessDraftSchema, type OrgAccess, type OrgAccessDraft } from "@/schemas/org-agent-access";

const fields = ["maintainers", "users"] as const;
const titles = {
  maintainers: "Who can maintain this specialist",
  users: "Who can chat with this specialist",
} as const;

export function OrgAgentMemberAccess({ orgId, agentId }: { orgId: string; agentId: string }) {
  useUiTranslation();
  const { policy, replace } = useOrgAgentAccess(orgId, agentId);
  if (policy.data === null) return null;
  if (!policy.data) return policy.isError
    ? <ErrorBanner message={translateUi("Could not load member access.")} onRetry={() => void policy.refetch()} />
    : <p role="status">{translateUi("Loading member access…")}</p>;
  return <PolicyEditor policy={policy.data} orgId={orgId} replace={replace}
    refresh={policy.refetch} loadFailed={policy.isError} />;
}

function PolicyEditor({ policy, orgId, replace, refresh, loadFailed }: {
  policy: OrgAccess; orgId: string; replace: ReturnType<typeof useOrgAgentAccess>["replace"];
  refresh: () => void; loadFailed: boolean;
}) {
  const members = useOrgMembers(orgId);
  const deniedRoster = members.error instanceof ApiError && (members.error.status === 403 || members.error.status === 404);
  useEffect(() => { if (deniedRoster) refresh(); }, [deniedRoster, refresh]);
  const eligible = (members.data ?? []).filter((m) => !m.revoked_at && (m.role === "admin" || m.role === "member"));
  const names = new Map(eligible.map((m) => [m.user_id, m.display_name || m.email || m.user_id]));
  const [page, setPage] = useState<keyof OrgAccessDraft>();
  const [query, setQuery] = useState("");
  const [base, setBase] = useState<number>();
  const [stale, setStale] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm<OrgAccessDraft>({ defaultValues: orgAccessDraftSchema.parse(policy), resolver: zodResolver(orgAccessDraftSchema) });
  const watched = form.watch();
  const draft = base === undefined || !policy.authoring_available ? policy : watched;
  const dirty = fields.some((field) => {
    const a = policy[field], b = draft[field];
    if (a === null || b === null) return a !== b;
    return a.length !== b.length || a.some((id) => !b.includes(id));
  });
  const conflict = base !== undefined && (stale || base !== policy.revision);
  const validPeople = fields.every((field) => draft[field]?.every((id) => names.has(id)) ?? true);
  const editable = policy.authoring_available && members.isSuccess && !loadFailed && !replace.isPending;
  const summary = (ids: string[] | null) => ids === null ? translateUi("All eligible organization members")
    : ids.length ? ids.map((id) => names.get(id) ?? id).join(", ") : translateUi("Nobody");
  function change(field: keyof OrgAccessDraft, value: string[] | null) {
    if (base === undefined) {
      form.reset(orgAccessDraftSchema.parse(policy));
      setBase(policy.revision);
    }
    setError(undefined);
    form.setValue(field, value);
  }
  function discard() {
    setBase(undefined);
    setStale(false);
    setError(undefined);
    form.reset(orgAccessDraftSchema.parse(policy));
  }
  const save = form.handleSubmit(async (values) => {
    if (!editable || !validPeople || conflict || base === undefined || !dirty) return;
    try {
      await replace.mutateAsync({ ...values, expected_revision: base });
      discard();
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 409) setStale(true);
      else setError(cause instanceof Error ? cause.message : "Could not save. Try again.");
    }
  });
  if (deniedRoster) return <ErrorBanner message={translateUi("Could not load members.")} onRetry={() => void members.refetch()} />;
  return <section aria-label={translateUi("Member access")}>
    <GroupedList title={translateUi("Member access")} footer={<>
      {!policy.authoring_available && <p>{translateUi("Editing is unavailable")} <Button type="button" variant="ghost" onClick={() => refresh()}>{translateUi("Retry")}</Button></p>}
      {members.isPending && <p role="status">{translateUi("Loading members…")}</p>}
      {members.isError && <ErrorBanner message={translateUi("Could not load members.")} onRetry={() => void members.refetch()} />}
      {loadFailed && <ErrorBanner message={translateUi("Could not load member access.")} onRetry={refresh} />}
      {error && <p role="alert">{translateUi(error)}</p>}
      {editable && !validPeople && <p role="alert">{translateUi("Remove people who are no longer eligible before saving.")}</p>}
    </>}>
      {fields.map((field) => {
        const title = translateUi(titles[field]);
        const ids = draft[field];
        const searchId = `member-access-${field}`;
        function open() { setQuery(""); setPage(field); }
        return <Fragment key={field}>
          <GroupedDisclosure data-drawer-page-trigger={searchId}
            search={{ id: searchId, title: titles[field], breadcrumb: "Overview › Member access", keywords: summary(ids) }}
            onClick={open} accessory={<span className="max-w-[45%] truncate type-secondary">{summary(ids)}</span>}>{title}</GroupedDisclosure>
          <GroupedDetail open={page === field} title={title} backLabel={translateUi("Member access")}
            searchPage={searchId} onReveal={open} onBack={() => setPage(undefined)}>
            <fieldset disabled={!editable} className="space-y-4">
              <Label asChild><legend className="sr-only">{title}</legend></Label>
              <GroupedList>
                {(["all", "selected"] as const).map((mode) => <GroupedRow asChild key={mode}>
                  <Label><input type="radio" name={searchId} checked={(ids === null) === (mode === "all")}
                    onChange={() => change(field, mode === "all" ? null : [])} />
                    {mode === "all" ? translateUi("All eligible organization members") : translateUi("Selected people")}
                  </Label>
                </GroupedRow>)}
              </GroupedList>
              {ids !== null && <GroupedList title={ids.length ? translateUi("Selected people") : translateUi("Nobody")}
                footer={translateUi("Select up to 128 people.")}>
                <GroupedRow><input type="search" aria-label={translateUi("Search members")} placeholder={translateUi("Search members")}
                  className="w-full bg-transparent" value={query} onChange={(e) => setQuery(e.target.value)} /></GroupedRow>
                {ids.filter((id) => !names.has(id)).map((id) => <GroupedRow asChild key={id}><Label>
                  <input type="checkbox" checked onChange={() => change(field, ids.filter((v) => v !== id))} />{id}
                </Label></GroupedRow>)}
                {eligible.filter((m) => [m.display_name, m.email, m.user_id].some((s) => s?.toLowerCase().includes(query.trim().toLowerCase())))
                  .map((m) => <GroupedRow asChild key={m.user_id}><Label>
                    <input type="checkbox" checked={ids.includes(m.user_id)} disabled={ids.length >= MAX_AGENT_ACCESS_PEOPLE && !ids.includes(m.user_id)}
                      onChange={(e) => change(field, e.target.checked ? [...ids, m.user_id] : ids.filter((id) => id !== m.user_id))} />
                    {names.get(m.user_id)} <span className="text-text-tertiary">{translateUi(m.role)}</span>
                  </Label></GroupedRow>)}
              </GroupedList>}
            </fieldset>
          </GroupedDetail>
        </Fragment>;
      })}
    </GroupedList>
    {policy.authoring_available && conflict && <GroupedList title={translateUi("Latest saved member access")}>
      {fields.map((field) => <GroupedRow key={field} title={translateUi(titles[field])} description={summary(policy[field])} />)}
      <GroupedRow><Button type="button" variant="outline" disabled={loadFailed || base === policy.revision}
        onClick={() => { setBase(policy.revision); setStale(false); }}>{translateUi("Reapply my draft")}</Button>
        <Button type="button" variant="ghost" onClick={discard}>{translateUi("Discard")}</Button></GroupedRow>
    </GroupedList>}
    <GroupedFormActions label={translateUi("Member access")} dirty={policy.authoring_available && dirty}
      pending={replace.isPending} disabled={!editable || !validPeople || conflict} save={save} discard={discard}
      changes={() => fields.map((field) => ({ label: translateUi(titles[field]), before: summary(policy[field]), after: summary(draft[field]) }))} />
  </section>;
}
