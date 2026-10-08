import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { useServiceCardView } from "@/stores/service-card-view-store";
import {
  DEFAULT_SERVICE_FILTERS,
  sameServiceFilters,
  serviceViewSchema,
  serviceViewsSchema,
  type ServiceViewsPreferences,
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

  const parsedViews = serviceViewsSchema.safeParse(
    user?.profile_config?.service_views,
  );
  const workspace: ServiceViewsPreferences = parsedViews.success
    ? parsedViews.data
    : {
        views: hasDefault
          ? [{ id: "legacy-default", name: "My default", filters: saved }]
          : [],
        default_id: hasDefault ? "legacy-default" : null,
      };
  const selectedViewId = sameAccount
    ? (view.savedViewId ?? workspace.default_id)
    : workspace.default_id;
  const viewsMutation = useMutation({
    mutationFn: async ({
      preferences,
      accountId,
    }: {
      preferences: ServiceViewsPreferences;
      accountId: string;
    }) => {
      await queryClient.cancelQueries({ queryKey: ["user", "me"] });
      if (useAuthStore.getState().user?.id !== accountId)
        throw new Error("Account changed. Please try again.");
      return serviceViewsSchema.parse(
        await api.put<unknown>(
          "/users/me/preferences/service-views",
          serviceViewsSchema.parse(preferences),
        ),
      );
    },
    onSuccess: (preferences, { accountId }) => {
      const current = useAuthStore.getState().user;
      if (current?.id !== accountId || !current.profile_config) return;
      const updated: User = {
        ...current,
        profile_config: {
          ...current.profile_config,
          service_views: preferences,
          services_view:
            preferences.views.find((view) => view.id === preferences.default_id)
              ?.filters ?? null,
        },
      };
      useAuthStore.getState().setUser(updated);
      queryClient.setQueryData(["user", "me"], updated);
    },
  });
  const canSaveViews = user?.profile_config?.service_views !== undefined;
  const persistViews = (
    preferences: ServiceViewsPreferences,
    onSuccess?: () => void,
  ) => {
    if (user && canSaveViews && !viewsMutation.isPending)
      viewsMutation.mutate({ accountId: user.id, preferences }, { onSuccess });
  };
  const viewsMutationForAccount =
    viewsMutation.variables?.accountId === user?.id;

  const setView = (update: {
    filters?: ServiceViewFilters;
    savedViewId?: string;
    expanded?: readonly string[];
  }) => {
    useServiceCardView.setState({
      accountId: user?.id,
      filters,
      expanded,
      savedViewId: selectedViewId ?? undefined,
      ...update,
    });
  };
  const mutationForAccount = mutation.variables?.accountId === user?.id;

  return {
    accountId: user?.id,
    workspace,
    selectedViewId,
    restoreSavedView: (id: string) => {
      const savedView = workspace.views.find(
        (savedView) => savedView.id === id,
      );
      if (savedView) setView({ filters: savedView.filters, savedViewId: id });
    },
    canSaveViews,
    saveNewView: (name: string, onSuccess?: () => void) =>
      persistViews(
        {
          ...workspace,
          views: [
            ...workspace.views,
            { id: crypto.randomUUID(), name, filters },
          ],
        },
        onSuccess,
      ),
    updateSavedView: (id: string) =>
      persistViews({
        ...workspace,
        views: workspace.views.map((view) =>
          view.id === id ? { ...view, filters } : view,
        ),
      }),
    setDefaultView: (id: string | null) =>
      persistViews({ ...workspace, default_id: id }),
    deleteSavedView: (id: string) =>
      persistViews({
        views: workspace.views.filter((view) => view.id !== id),
        default_id: workspace.default_id === id ? null : workspace.default_id,
      }),
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
    isSaving:
      (mutationForAccount && mutation.isPending) ||
      (viewsMutationForAccount && viewsMutation.isPending),
    saveError:
      (mutationForAccount && mutation.error) ||
      (viewsMutationForAccount && viewsMutation.error)
        ? "Could not save your views. Please try again."
        : null,
  };
}
