import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { erasureProgressSchema } from "@/schemas/account-erasure";
import { Button } from "@/components/ui/button";

export function AccountErasureProgress({ userId }: { readonly userId: string }) {
  return <AccountErasurePages key={userId} userId={userId} />;
}

function AccountErasurePages({ userId }: { readonly userId: string }) {
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
  if (progress.isPending) return <p className="text-sm text-muted-foreground">Loading erasure evidence…</p>;
  if (progress.error) return <section aria-label="Account erasure progress"><p role="alert">{progress.error instanceof ApiError ? progress.error.message : "Erasure progress is unavailable."}</p><Button variant="outline" onClick={() => void progress.refetch()}>Retry loading progress</Button></section>;
  if (!progress.data || (!progress.data.erasure && !progress.data.effects.length)) return null;
  const { erasure, effects, next_after: next } = progress.data;
  return <section className="space-y-3 rounded-lg border p-4" aria-label="Account erasure progress">
    <h3 className="font-semibold">Account erasure progress</h3>
    <p className="text-sm">{erasure?.status === "complete" ? "Required erasure work has verified completion. Policy exceptions and retained evidence are listed below." : erasure ? "Deletion accepted. Sign-in is closed; required local and remote work continues until verified." : "Provider revocation and issuance outcomes are listed below. No account deletion has been requested."}</p>
    {erasure ? <ul className="space-y-2 text-sm">{erasure.domains.map((domain) => <li key={domain.domain}>
      {domain.domain.replaceAll("_", " ")}: {domain.status.replaceAll("_", " ")}{domain.reason ? ` (${domain.reason.replaceAll("_", " ")})` : ""}
      {domain.status === "operator_action" ? <Button className="ml-2" variant="outline" disabled={retry.isPending || !erasure.job_id || domain.retry_generation === undefined} onClick={() => { if (erasure.job_id && domain.retry_generation !== undefined) retry.mutate({ domain: domain.domain, job_id: erasure.job_id, expected_retry_generation: domain.retry_generation, idempotency_key: crypto.randomUUID() }); }}>Retry domain</Button> : null}
    </li>)}</ul> : null}
    {retry.error ? <p role="alert">{retry.error instanceof ApiError ? retry.error.message : "Domain retry failed."}</p> : null}
    {progress.data.node_drains.length ? <><h4 className="text-sm font-medium">Outstanding node drains</h4><ul className="space-y-1 text-sm">{progress.data.node_drains.map((node) => <li key={node.node_id}><code>{node.node_id}</code>: awaiting connection close{node.fence ? ` on ${node.fence.instance_name}, generation ${node.fence.generation_id}` : ""}</li>)}</ul><p className="text-sm text-muted-foreground">Unavailable replicas retain their drain obligation. An operator may reconcile an exact generation only with evidence that the old process has stopped.</p></> : null}
    <div className="flex gap-2">{nodesAfter ? <Button variant="outline" onClick={() => setNodesAfter(null)}>First nodes page</Button> : null}{progress.data.next_node_after ? <Button variant="outline" onClick={() => setNodesAfter(progress.data.next_node_after)}>Next nodes page</Button> : null}</div>
    {effects.length ? <><h4 className="text-sm font-medium">Provider effect outcomes</h4><ul className="space-y-1 text-sm">{effects.map((effect) => <li key={effect._id}><code>{effect._id}</code>: {effect.status.replaceAll("_", " ")}{effect.reason ? ` (${effect.reason.replaceAll("_", " ")})` : ""}{effect.retry_generation !== undefined ? `; retry generation ${effect.retry_generation}` : ""}{effect.last_reconciliation_request_digest ? <span className="block break-all text-xs text-muted-foreground">Last reconciliation request: <code>{effect.last_reconciliation_request_digest}</code></span> : null}</li>)}</ul><p className="text-sm text-muted-foreground">An authorized operator must record effect-specific evidence through the erasure reconciliation API. Retry alone never attests remote completion. For a single-attempt revocation, any authorization granted since at the provider for this OAuth app will also be revoked.</p></> : null}
    <div className="flex gap-2">{after ? <Button variant="outline" onClick={() => setAfter(null)}>First effects page</Button> : null}{next ? <Button variant="outline" onClick={() => setAfter(next)}>Next effects page</Button> : null}</div>
  </section>;
}
