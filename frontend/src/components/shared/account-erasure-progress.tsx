import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { erasureProgressSchema } from "@/schemas/account-erasure";
import { Button } from "@/components/ui/button";

export function AccountErasureProgress({ userId }: { readonly userId: string }) {
  useUiTranslation();
  return <AccountErasurePages key={userId} userId={userId} />;
}

function AccountErasurePages({ userId }: { readonly userId: string }) {
  useUiTranslation();
  const [after, setAfter] = useState<string | null>(null);
  const [nodesAfter, setNodesAfter] = useState<string | null>(null);
  const client = useQueryClient();
  const progress = useQuery({
    queryKey: ["admin", "erasure", userId, after, nodesAfter],
    queryFn: async () => erasureProgressSchema.parse(await api.get<unknown>(`/admin/users/${encodeURIComponent(userId)}/erasure?${new URLSearchParams({...(after ? {after} : {}),...(nodesAfter ? {nodes_after:nodesAfter} : {})}).toString()}`)),
    refetchInterval: 5000,
  });
  const retry = useMutation({
    mutationFn: (request: { domain: string; job_id: string; expected_retry_generation: number; idempotency_key: string }) => api.post(`/admin/users/${encodeURIComponent(userId)}/erasure/retry`, request),
    onSuccess: () => client.invalidateQueries({ queryKey: ["admin", "erasure", userId] }),
  });
  if (progress.isPending) return <p className="text-14 text-muted-foreground">{translateUi("Loading erasure evidence…")}</p>;
  if (progress.error) return <section aria-label={translateUi("Account erasure progress")}><p role="alert">{progress.error instanceof ApiError ? progress.error.message : translateUi("Erasure progress is unavailable.")}</p><Button variant="outline" onClick={() => void progress.refetch()}>{translateUi("Retry loading progress")}</Button></section>;
  if (!progress.data || (!progress.data.erasure && !progress.data.effects.length)) return null;
  const { erasure, effects, next_after: next } = progress.data;
  return <section className="space-y-3 rounded-lg border p-4" aria-label={translateUi("Account erasure progress")}>
    <h3 className="font-semibold">{translateUi("Account erasure progress")}</h3>
    <p className="text-14">{erasure?.status === "complete" ? translateUi("Required erasure work has verified completion. Policy exceptions and retained evidence are listed below.") : erasure ? translateUi("Deletion accepted. Sign-in is closed; required local and remote work continues until verified.") : translateUi("Provider revocation and issuance outcomes are listed below. No account deletion has been requested.")}</p>
    {erasure ? <ul className="space-y-2 text-14">{erasure.domains.map((domain) => <li key={domain.domain}>
      {domain.domain.replaceAll("_", " ")}: {domain.status.replaceAll("_", " ")}{domain.reason ? ` (${domain.reason.replaceAll("_", " ")})` : ""}
      {domain.status === "operator_action" ? <Button className="ml-2" variant="outline" disabled={retry.isPending || !erasure.job_id || domain.retry_generation === undefined} onClick={() => { if (erasure.job_id && domain.retry_generation !== undefined) retry.mutate({ domain: domain.domain, job_id: erasure.job_id, expected_retry_generation: domain.retry_generation, idempotency_key: crypto.randomUUID() }); }}>{translateUi("Retry domain")}</Button> : null}
    </li>)}</ul> : null}
    {retry.error ? <p role="alert">{retry.error instanceof ApiError ? retry.error.message : translateUi("Domain retry failed.")}</p> : null}
    {progress.data.node_drains.length ? <><h4 className="type-row-title">{translateUi("Outstanding node drains")}</h4><ul className="space-y-1 text-14">{progress.data.node_drains.map((node) => <li key={node.node_id}><code>{node.node_id}</code>{translateUi(": awaiting connection close")}{node.fence ? translateUi(" on {{value1}}, generation {{value2}}", { value1: node.fence.instance_name, value2: node.fence.generation_id }) : ""}</li>)}</ul><p className="text-14 text-muted-foreground">{translateUi("Unavailable replicas retain their drain obligation. An operator may reconcile an exact generation only with evidence that the old process has stopped.")}</p></> : null}
    <div className="flex gap-2">{nodesAfter ? <Button variant="outline" onClick={() => setNodesAfter(null)}>{translateUi("First nodes page")}</Button> : null}{progress.data.next_node_after ? <Button variant="outline" onClick={() => setNodesAfter(progress.data.next_node_after)}>{translateUi("Next nodes page")}</Button> : null}</div>
    {effects.length ? <><h4 className="type-row-title">{translateUi("Provider effect outcomes")}</h4><ul className="space-y-1 text-14">{effects.map((effect) => <li key={effect._id}><code>{effect._id}</code>: {effect.status.replaceAll("_", " ")}{effect.reason ? ` (${effect.reason.replaceAll("_", " ")})` : ""}{effect.retry_generation !== undefined ? translateUi("; retry generation {{value1}}", { value1: effect.retry_generation }) : ""}{effect.last_reconciliation_request_digest ? <span className="block break-all text-13 text-muted-foreground">{translateUi("Last reconciliation request: ")}<code>{effect.last_reconciliation_request_digest}</code></span> : null}</li>)}</ul><p className="text-14 text-muted-foreground">{translateUi("An authorized operator must record effect-specific evidence through the erasure reconciliation API. Retry alone never attests remote completion. For a single-attempt revocation, any authorization granted since at the provider for this OAuth app will also be revoked.")}</p></> : null}
    <div className="flex gap-2">{after ? <Button variant="outline" onClick={() => setAfter(null)}>{translateUi("First effects page")}</Button> : null}{next ? <Button variant="outline" onClick={() => setAfter(next)}>{translateUi("Next effects page")}</Button> : null}</div>
  </section>;
}
