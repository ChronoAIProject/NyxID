/* eslint-disable react-refresh/only-export-components -- the provider and its typed query hooks form one runtime boundary. */
import { createContext, useContext, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { crmQueryKeys, useCrmAggregate } from "@/hooks/use-crm";
import type {
  CrmNamedAggregateQuery,
  CrmNamedQuery,
  CrmNamedTableQuery,
} from "@/lib/crm/views/schema";
import { useAuthStore } from "@/stores/auth-store";

interface CrmViewQueryContextValue {
  readonly ownerId?: string;
  readonly queries: Readonly<Record<string, CrmNamedQuery>>;
}

const CrmViewQueryContext = createContext<CrmViewQueryContextValue | null>(
  null,
);

export function CrmViewQueryProvider({
  ownerId,
  queries,
  children,
}: CrmViewQueryContextValue & { readonly children: ReactNode }) {
  return (
    <CrmViewQueryContext.Provider value={{ ownerId, queries }}>
      {children}
    </CrmViewQueryContext.Provider>
  );
}

function useCrmViewQueryContext() {
  const context = useContext(CrmViewQueryContext);
  if (!context) {
    throw new Error(
      "CRM saved-view components must render inside CrmViewQueryProvider",
    );
  }
  return context;
}

export function useCrmNamedTableQuery(queryId: string): {
  readonly ownerId?: string;
  readonly namedQuery?: CrmNamedTableQuery;
} {
  const context = useCrmViewQueryContext();
  const query = context.queries[queryId];
  return {
    ownerId: context.ownerId,
    namedQuery: query?.kind === "table" ? query : undefined,
  };
}

export function useCrmNamedAggregateQuery(queryId: string) {
  const context = useCrmViewQueryContext();
  const query = context.queries[queryId];
  const namedQuery: CrmNamedAggregateQuery | undefined =
    query?.kind === "aggregate" ? query : undefined;
  const result = useCrmAggregate(
    namedQuery?.query,
    context.ownerId,
    Boolean(namedQuery),
  );
  return { namedQuery, result };
}

export function useInvalidateCrmViewQueries() {
  const context = useCrmViewQueryContext();
  const actor = useAuthStore((state) => state.user?.id);
  const owner = context.ownerId ?? actor;
  const queryClient = useQueryClient();
  return () =>
    queryClient.invalidateQueries({ queryKey: crmQueryKeys.root(owner) });
}

export function useCrmViewOwnerId() {
  return useCrmViewQueryContext().ownerId;
}
