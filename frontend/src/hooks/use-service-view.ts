import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { useServiceCardView } from "@/stores/service-card-view-store";
import {
  DEFAULT_SERVICE_FILTERS,
  sameServiceFilters,
  serviceViewSchema,
  type ServiceViewFilters,
} from "@/schemas/service-view";
import type { User } from "@/types/api";

export function useServiceView() {
  const user = useAuthStore((state) => state.user);
  const queryClient = useQueryClient();
  const view = useServiceCardView();
  const parsed = serviceViewSchema.safeParse(
    user?.profile_config?.services_view,
  );
  const saved = parsed.success ? parsed.data : DEFAULT_SERVICE_FILTERS;
  const sameAccount = view.accountId === user?.id;
  const draftFilters = serviceViewSchema.safeParse(
    sameAccount ? view.filters : undefined,
  );
  const filters = draftFilters.success ? draftFilters.data : saved;
  const expanded = sameAccount ? view.expanded.slice(-1) : [];
  const canSave = user?.profile_config?.services_view !== undefined;
  const hasDefault = parsed.success;

  const mutation = useMutation({
    mutationFn: async ({
      filters,
      accountId,
    }: {
      filters: ServiceViewFilters;
      accountId: string;
    }) => {
      await queryClient.cancelQueries({ queryKey: ["user", "me"] });
      if (useAuthStore.getState().user?.id !== accountId)
        throw new Error("Account changed. Please try again.");
      return serviceViewSchema.parse(
        await api.put<unknown>(
          "/users/me/preferences/services",
          serviceViewSchema.parse(filters),
        ),
      );
    },
    onSuccess: (savedFilters, { accountId }) => {
      const current = useAuthStore.getState().user;
      if (current?.id !== accountId || !current.profile_config) return;
      const updated: User = {
        ...current,
        profile_config: {
          ...current.profile_config,
          services_view: savedFilters,
        },
      };
      useAuthStore.getState().setUser(updated);
      queryClient.setQueryData(["user", "me"], updated);
    },
  });

  const setView = (update: {
    filters?: ServiceViewFilters;
    expanded?: readonly string[];
  }) => {
    useServiceCardView.setState({
      accountId: user?.id,
      filters,
      expanded,
      ...update,
    });
  };
  const mutationForAccount = mutation.variables?.accountId === user?.id;

  return {
    accountId: user?.id,
    filters,
    expanded,
    canSave,
    hasDefault,
    savedFilters: saved,
    isDefault: hasDefault && sameServiceFilters(filters, saved),
    differsFromDefault: !sameServiceFilters(filters, saved),
    setFilters: (filters: ServiceViewFilters) => setView({ filters }),
    setExpanded: (expanded: readonly string[]) =>
      setView({ expanded: expanded.slice(-1) }),
    restoreDefault: () => setView({ filters: saved }),
    saveDefault: () => {
      if (user && canSave) mutation.mutate({ accountId: user.id, filters });
    },
    isSaving: mutationForAccount && mutation.isPending,
    saveError:
      mutationForAccount && mutation.error
        ? "Could not save your default view. Please try again."
        : null,
  };
}
