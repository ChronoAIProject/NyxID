import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { machineGrantUnavailable, useMachineGrant, useMachineGrantList } from "@/hooks/use-machine-grants";
import type { MachineGrant } from "@/schemas/machine-grants";
import { formatDate } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "./error-banner";

function reasonLabel(reason: string | null | undefined): string {
  switch (reason) {
    case "missing": return "Not found";
    case "disabled": return "Disabled";
    case "expired": return "Expired";
    case "revoked": return "Revoked";
    case "changed": return "Changed";
    case "ineligible": return "Not eligible";
    default: return "Unknown";
  }
}

function liveLabel(valid: boolean | undefined, reason: string | null | undefined): string {
  if (valid === true) return "Valid";
  if (reason) return reasonLabel(reason);
  return "Unknown";
}

function GrantValidity({ grant, active }: { readonly grant: MachineGrant; readonly active: boolean }) {
  const { t } = useUiTranslation();
  const [requested, setRequested] = useState(false);
  const source = grant.source.kind === "api_key"
    ? { kind: "api_key" as const, id: grant.source.api_key_id }
    : { kind: "oauth" as const, id: grant.source.oauth_client_id };
  const query = useMachineGrant(source, requested);
  const sameGrant = query.data?.grant?.id === grant.id && query.data.grant.revision === grant.revision;
  const verified = requested && !query.error && !query.isFetching && sameGrant;
  const independentValidity = query.data?.source_valid !== undefined && query.data?.grant_valid !== undefined;
  const sourceValid = query.data?.source_valid;
  const grantValid = active && query.data?.grant_valid;
  const grantReason = grant.status === "revoked" ? "revoked" : !active ? "expired" : query.data?.grant_validity_reason;
  const combinedValid = active && query.data?.source_and_grant_valid === true && (!independentValidity || (sourceValid === true && grantValid === true));
  return <div className="space-y-2">
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant={verified && combinedValid ? "success" : "secondary"}>{t("Source + grant: ")}{!requested ? t("Not checked") : query.isFetching ? t("Checking") : !verified ? t("Unknown") : combinedValid ? t("Valid") : t("Not currently valid")}</Badge>
      {verified && independentValidity && <><Badge variant={sourceValid ? "success" : "warning"}>{t("Source: ")}{t(liveLabel(sourceValid, query.data?.source_validity_reason))}</Badge><Badge variant={grantValid ? "success" : "warning"}>{t("Grant validity: ")}{t(liveLabel(grantValid, grantReason))}</Badge></>}
      <Button variant="outline" size="sm" disabled={query.isFetching} onClick={() => { if (requested) void query.refetch(); else setRequested(true); }}>
        {requested ? t("Recheck source and grant") : t("Check source and grant")}
      </Button>
    </div>
    {requested && query.error && <p role="alert" className="text-12 text-warning">{t("Could not verify this source and grant. Its current validity is unknown; check the source page or retry.")}</p>}
    {requested && !query.error && !query.isFetching && query.data && !sameGrant && <p role="alert" className="text-12 text-warning">{t("This grant changed or was replaced. Refresh the grant list before relying on its permissions.")}</p>}
    <p className="text-11 text-muted-foreground">{independentValidity ? t("Source authentication and Machine grant validity are live checks over their authoritative records. Unknown results do not establish execution authority, Machine readiness, or the client’s local token state.") : t("The source page reports one combined source and grant check; it does not report source validity separately. This check does not establish the client’s local token state or Machine readiness.")}</p>
  </div>;
}

export function MachineGrantList({ machineId }: { readonly machineId: string }) {
  const { t } = useUiTranslation();
  const query = useMachineGrantList(machineId);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  return <section aria-labelledby="external-machine-grants" className="space-y-3">
    <div className="flex items-center justify-between gap-3"><h3 id="external-machine-grants" className="text-15 font-semibold">{t("External subject permissions")}</h3><Button variant="outline" size="sm" onClick={() => void query.refetch()}>{t("Refresh grants")}</Button></div>
    <p className="text-12 text-muted-foreground">{t("Each row is this account’s real OAuth client or parent Key. Child login credentials share their parent’s grant and task view. Labels and MCP connection names do not create independent permissions. Historical and revoked grants remain visible. Use Load older grants when the server provides another page.")}</p>
    {query.isPending && <p role="status">{t("Loading external Machine permissions…")}</p>}
    {query.error && <ErrorBanner message={machineGrantUnavailable(query.error) ? t("The per-Machine grant view is not available on this server yet. Manage permissions from the original Key or Authorized Apps page; no empty authorization state has been inferred.") : t("Could not load external Machine permissions. Retry before relying on this view.")} onRetry={() => void query.refetch()} />}
    {!query.error && query.data && query.data.next_cursor === undefined && <p role="status" className="text-12 text-warning">{t("This server has not provided grant pagination. This list may be incomplete; manage a known subject from its parent Key or Authorized Apps page.")}</p>}
    {!query.error && query.data?.external_grants.length === 0 && <p className="rounded-lg border border-border p-3 text-12 text-muted-foreground">{t("No external grants were returned for this Machine. Assistant Agent assignments above are managed independently.")}</p>}
    {!query.error && query.data?.external_grants.map((grant) => {
      const assignment = grant.assignments[machineId];
      const expired = new Date(grant.expires_at).getTime() <= now;
      return <article key={grant.id} className="space-y-3 rounded-lg border border-border bg-card p-4">
        <div className="flex flex-wrap items-center gap-2"><h4 className="text-13 font-medium">{grant.source.kind === "api_key" ? t("Parent Key") : t("OAuth client")}</h4><Badge variant={grant.status === "revoked" || expired ? "warning" : "secondary"}>{grant.status === "revoked" ? t("Revoked") : expired ? t("Expired") : t("Active grant")}</Badge><span className="text-11 text-muted-foreground">{t("Revision ")}{grant.revision}{t(" · policy ")}{grant.policy_revision}</span></div>
        <p className="break-all font-mono text-11">{grant.source.kind === "api_key" ? grant.source.api_key_id : grant.source.oauth_client_id}</p>
        <p className="text-11 text-muted-foreground">{t("Expires ")}{formatDate(grant.expires_at)}{t(". Current source authentication and execution authority are revalidated on every use; an active grant alone does not establish either.")}</p>
        <GrantValidity key={`${machineId}-${grant.id}-${grant.revision}`} grant={grant} active={grant.status === "active" && !expired} />
        {assignment ? <dl className="grid gap-3 text-12 sm:grid-cols-2"><div><dt className="text-muted-foreground">{t("Capabilities / mode")}</dt><dd>{[assignment.capabilities.shell && "Commands", assignment.capabilities.files && "Files"].filter(Boolean).join(" · ") || t("None")} · {assignment.mode}</dd></div><div><dt className="text-muted-foreground">{t("Assignment revision")}</dt><dd>{assignment.revision}</dd></div><div><dt className="text-muted-foreground">{t("File-tool roots")}</dt><dd className="break-all">{assignment.file_roots.join(", ") || t("None")}</dd></div><div><dt className="text-muted-foreground">{t("Gateway service ceiling")}</dt><dd className="break-all">{assignment.service_ids.join(", ") || t("None")}</dd></div></dl> : <p role="alert" className="text-12 text-warning">{t("This returned grant no longer includes this Machine. Refresh the view.")}</p>}
        <p className="text-11 text-muted-foreground">{t("Changes or revocation affect this subject’s earlier work across all Machines in its grant. File roots do not isolate shell access. Revocation leaves the Machine and other subjects’ permissions intact.")}</p>
        {grant.source.kind === "api_key" ? <Link to="/keys/api-key/$keyId" params={{ keyId: grant.source.api_key_id }} className="text-12 text-primary-text hover:underline">{t("Manage parent Key and login credentials")}</Link> : <Link to="/settings/consents" search={{ tab: "apps" }} className="text-12 text-primary-text hover:underline">{t("Manage OAuth consent and Machine permissions")}</Link>}
      </article>;
    })}
    {!query.error && query.hasNextPage && <Button variant="outline" onClick={() => void query.fetchNextPage()} disabled={query.isFetchingNextPage}>{query.isFetchingNextPage ? t("Loading older grants…") : t("Load older grants")}</Button>}
  </section>;
}
