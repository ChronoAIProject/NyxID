import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  aurinkoAuthorizationSchema,
  aurinkoMailboxesSchema,
  type AurinkoProvider,
} from "@/schemas/aurinko-mailboxes";

const ROOT = "/providers/aurinko/mailboxes";

export async function listAurinkoMailboxes(ownerId?: string | null) {
  const query = ownerId ? `?${new URLSearchParams({ owner_id: ownerId })}` : "";
  return aurinkoMailboxesSchema.parse(await api.get(`${ROOT}${query}`))
    .mailboxes;
}

export function useAurinkoMailboxes(ownerId?: string | null, enabled = true) {
  return useQuery({
    queryKey: ["aurinko-mailboxes", ownerId ?? "personal"],
    queryFn: () => listAurinkoMailboxes(ownerId),
    enabled,
    retry: false,
    staleTime: 0,
  });
}

export async function authorizeAurinkoMailbox(input: {
  readonly provider: AurinkoProvider;
  readonly label: string;
  readonly owner_id?: string;
  readonly connection_id?: string;
}) {
  return aurinkoAuthorizationSchema.parse(
    await api.post(`${ROOT}/authorize`, input),
  );
}

export async function cancelAurinkoAuthorization(nonce: string) {
  await api.delete(`${ROOT}/attempts/${encodeURIComponent(nonce)}`);
}
