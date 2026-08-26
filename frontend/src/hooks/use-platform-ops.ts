import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY,
  PLATFORM_VENDOR_TEMPLATES_QUERY_KEY,
  platformOperationListSchema,
  platformOperationSchema,
  platformVendorRequirementSchema,
  platformVendorRequirementListSchema,
  type PlatformVendorRequirementList,
  type PlatformVendorTemplateForm,
  type PlatformVendorTemplateInput,
  type ProvisionPlatformVendorVariables,
  type PlatformOperation,
  type PlatformOperationList,
  type UpdatePlatformOperationVariables,
} from "@/schemas/platform-ops";
import type { DownstreamService } from "@/types/api";

export function usePlatformOperations() {
  return useQuery({
    queryKey: PLATFORM_OPERATION_QUERY_KEY,
    queryFn: async (): Promise<PlatformOperationList> => {
      const response = await api.get<unknown>("/admin/platform-ops");
      return platformOperationListSchema.parse(response);
    },
  });
}

export function useUpdatePlatformOperation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      op,
      data,
    }: UpdatePlatformOperationVariables): Promise<PlatformOperation> => {
      const response = await api.put<unknown>(
        `/admin/platform-ops/${op}`,
        data,
      );
      return platformOperationSchema.parse(response);
    },
    onSuccess: (updated) => {
      queryClient.setQueryData<PlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
        (current) => {
          if (!current) return current;
          return {
            operations: current.operations.map((operation) =>
              operation.op === updated.op ? updated : operation,
            ),
          };
        },
      );
    },
  });
}

export function usePlatformVendorRequirements() {
  return useQuery({
    queryKey: PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY,
    queryFn: async (): Promise<PlatformVendorRequirementList> => {
      const response = await api.get<unknown>(
        "/admin/platform-ops/vendor-requirements",
      );
      return platformVendorRequirementListSchema.parse(response);
    },
  });
}

export function usePlatformVendorTemplates() {
  return useQuery({
    queryKey: PLATFORM_VENDOR_TEMPLATES_QUERY_KEY,
    queryFn: async (): Promise<PlatformVendorRequirementList> => {
      const response = await api.get<unknown>(
        "/admin/platform-ops/vendor-templates",
      );
      return platformVendorRequirementListSchema.parse(response);
    },
  });
}

export function useCreatePlatformVendorTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (data: PlatformVendorTemplateInput) => {
      const response = await api.post<unknown>(
        "/admin/platform-ops/vendor-templates",
        data,
      );
      return platformVendorRequirementSchema.parse(response);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_TEMPLATES_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY });
    },
  });
}

export function useUpdatePlatformVendorTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      id,
      data,
    }: {
      readonly id: string;
      readonly data: PlatformVendorTemplateForm;
    }) => {
      const response = await api.put<unknown>(
        `/admin/platform-ops/vendor-templates/${id}`,
        data,
      );
      return platformVendorRequirementSchema.parse(response);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_TEMPLATES_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY });
    },
  });
}

export function useDisablePlatformVendorTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      await api.delete<void>(`/admin/platform-ops/vendor-templates/${id}`);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_TEMPLATES_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY });
    },
  });
}

export function useProvisionPlatformVendor() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      requirement,
      data,
      replaceServiceId,
    }: ProvisionPlatformVendorVariables): Promise<DownstreamService> => {
      if (data.vendor !== requirement.vendor) {
        throw new Error("Selected vendor does not match its requirement");
      }
      if (replaceServiceId) {
        await api.delete<void>(`/services/${replaceServiceId}`);
      }

      try {
        return await api.post<DownstreamService>("/services", {
          name: `Platform ${requirement.display_name}`,
          slug: requirement.slug,
          service_type: "http",
          base_url: requirement.base_url,
          auth_method: requirement.auth_method,
          ...(requirement.auth_key_name
            ? { auth_key_name: requirement.auth_key_name }
            : {}),
          credential: data.credential,
          service_category: requirement.service_category,
          visibility: requirement.visibility,
          ...(data.note ? { auth_notes: data.note } : {}),
        });
      } catch (error) {
        if (replaceServiceId) {
          const detail = error instanceof Error ? ` ${error.message}` : "";
          throw new Error(
            `The existing row was deactivated, but the corrected row could not be created.${detail}`,
          );
        }
        throw error;
      }
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY,
      });
    },
  });
}
