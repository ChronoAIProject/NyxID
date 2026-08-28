import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
  adminPlatformOperationSchema,
  platformOperationListSchema,
  platformOperationDiscoveryListSchema,
  platformOperationSchema,
  type AdminPlatformOperation,
  type PlatformOperation,
  type PlatformOperationList,
  type PlatformOperationDiscoveryList,
  type UpdatePlatformOperationVariables,
} from "@/schemas/platform-ops";

function constrainedOperationForCards(
  operation: AdminPlatformOperation,
): PlatformOperation | null {
  if (operation.kind.type !== "constrained") return null;
  const common = {
    operation_id: operation.operation_id,
    enabled: operation.enabled,
    vendor_service_slug:
      operation.provider_slug ?? "deleted-platform-provider",
    updated_at: operation.updated_at,
    updated_by: operation.updated_by,
    vendor_service_id: operation.catalog_service_id,
    pricing: operation.pricing,
  };
  const caps = operation.limits.per_request;
  const daily = operation.limits.per_user_per_day;
  const config = operation.kind.config;
  if (
    operation.kind.op === "speak" &&
    config.type === "speak" &&
    caps.type === "speak" &&
    daily !== null
  ) {
    return {
      op: "speak",
      ...common,
      config: {
        type: "speak",
        allowed_voice_ids: config.allowed_voice_ids,
        max_chars: caps.max_chars,
        model_id: config.model_id,
        max_calls_per_user_per_day: daily,
      },
    };
  }
  if (
    operation.kind.op === "call_and_say" &&
    config.type === "call_and_say" &&
    caps.type === "call_and_say" &&
    daily !== null
  ) {
    return {
      op: "call_and_say",
      ...common,
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: config.allowed_destination_prefixes,
        max_message_chars: caps.max_message_chars,
        max_duration_seconds: caps.max_duration_seconds,
        voice: config.voice,
        max_calls_per_user_per_day: daily,
        account_sid: config.account_sid,
        call_from: config.call_from,
      },
    };
  }
  if (
    operation.kind.op === "flight_search" &&
    config.type === "flight_search" &&
    caps.type === "flight_search" &&
    daily !== null
  ) {
    return {
      op: "flight_search",
      ...common,
      config: {
        type: "flight_search",
        max_offers_cap: caps.max_offers,
        max_searches_per_user_per_day: daily,
      },
    };
  }
  return null;
}

function billingPayload(operation: PlatformOperation) {
  return {
    metric: operation.pricing.metric,
    price_per_unit: operation.pricing.price_per_unit,
    secondary: operation.pricing.secondary
      ? {
          metric: operation.pricing.secondary.metric,
          price_per_unit: operation.pricing.secondary.price_per_unit,
        }
      : null,
    base_fee_per_call: operation.pricing.base_fee_per_call,
  };
}

function updatePayload(
  operation: PlatformOperation,
  variables: UpdatePlatformOperationVariables,
) {
  const { data } = variables;
  const common = {
    enabled: data.enabled,
    billing: billingPayload(operation),
  };
  switch (variables.op) {
    case "speak":
      return {
        ...common,
        kind: {
          kind: "constrained" as const,
          op: "speak" as const,
          config: {
            type: "speak" as const,
            allowed_voice_ids: variables.data.config.allowed_voice_ids,
            model_id: variables.data.config.model_id,
            max_calls_per_user_per_day:
              variables.data.config.max_calls_per_user_per_day,
          },
        },
        limits: {
          per_request: {
            type: "speak" as const,
            max_chars: variables.data.config.max_chars,
          },
          per_user_per_day:
            variables.data.config.max_calls_per_user_per_day,
        },
      };
    case "call_and_say":
      return {
        ...common,
        kind: {
          kind: "constrained" as const,
          op: "call_and_say" as const,
          config: {
            type: "call_and_say" as const,
            allowed_destination_prefixes:
              variables.data.config.allowed_destination_prefixes,
            voice: variables.data.config.voice,
            account_sid: variables.data.config.account_sid,
            call_from: variables.data.config.call_from,
          },
        },
        limits: {
          per_request: {
            type: "call_and_say" as const,
            max_message_chars: variables.data.config.max_message_chars,
            max_duration_seconds:
              variables.data.config.max_duration_seconds,
          },
          per_user_per_day:
            variables.data.config.max_calls_per_user_per_day,
        },
      };
    case "flight_search":
      return {
        ...common,
        kind: {
          kind: "constrained" as const,
          op: "flight_search" as const,
          config: { type: "flight_search" as const },
        },
        limits: {
          per_request: {
            type: "flight_search" as const,
            max_offers: variables.data.config.max_offers_cap,
          },
          per_user_per_day:
            variables.data.config.max_searches_per_user_per_day,
        },
      };
  }
}

export function usePlatformOperations() {
  return useQuery({
    queryKey: PLATFORM_OPERATION_QUERY_KEY,
    queryFn: async (): Promise<PlatformOperationList> => {
      const response = await api.get<unknown>("/admin/platform-ops");
      const parsed = platformOperationListSchema.parse(response);
      return {
        operations: parsed.operations
          .map(constrainedOperationForCards)
          .filter((operation): operation is PlatformOperation => operation !== null),
      };
    },
  });
}

export function usePlatformOperationDiscovery(enabled = true) {
  return useQuery({
    queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    enabled,
    queryFn: async (): Promise<PlatformOperationDiscoveryList> => {
      const response = await api.get<unknown>("/platform-ops");
      return platformOperationDiscoveryListSchema.parse(response);
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
      const current = queryClient
        .getQueryData<PlatformOperationList>(PLATFORM_OPERATION_QUERY_KEY)
        ?.operations.find((operation) => operation.op === op);
      if (!current?.operation_id) {
        throw new Error("Platform operation ID is unavailable");
      }
      const response = await api.put<unknown>(
        `/admin/platform-ops/${current.operation_id}`,
        updatePayload(current, { op, data } as UpdatePlatformOperationVariables),
      );
      const updated = constrainedOperationForCards(
        adminPlatformOperationSchema.parse(response),
      );
      if (!updated) {
        throw new Error("Updated operation is not a constrained operation");
      }
      return platformOperationSchema.parse(updated);
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
