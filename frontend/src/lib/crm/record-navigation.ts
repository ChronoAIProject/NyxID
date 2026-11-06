import { createContext, useContext } from "react";

export const CrmRecordNavigation = createContext<
  ((id: string) => void) | undefined
>(undefined);
export function useCrmRecordNavigation() {
  return useContext(CrmRecordNavigation);
}

export function crmLink({
  ownerId,
  recordId,
  viewId,
  object,
}: {
  ownerId?: string;
  recordId?: string;
  viewId?: string;
  object?: string;
}) {
  const params = new URLSearchParams();
  if (ownerId) params.set("owner", ownerId);
  if (recordId) params.set("record", recordId);
  if (viewId) params.set("view", viewId);
  if (object) params.set("object", object);
  return `/assistant/crm${params.size ? `?${params}` : ""}`;
}
