import type { ComponentType } from "react";
import { AssistantKeyBindDialog } from "@/components/assistant/assistant-key-bind-dialog";
import { AssistantKeyCreateDialog } from "@/components/assistant/assistant-key-create-dialog";
import { AssistantKeyDeleteDialog } from "@/components/assistant/assistant-key-delete-dialog";
import { AssistantKeyRotateDialog } from "@/components/assistant/assistant-key-rotate-dialog";
import { AssistantKeyScopeDialog } from "@/components/assistant/assistant-key-scope-dialog";
import { AssistantKeyUpdateDialog } from "@/components/assistant/assistant-key-update-dialog";
import { AssistantServiceDeleteDialog } from "@/components/assistant/assistant-service-delete-dialog";
import { AssistantServiceRotateCredentialDialog } from "@/components/assistant/assistant-service-rotate-credential-dialog";
import { AssistantServiceRouteDialog } from "@/components/assistant/assistant-service-route-dialog";
import { AssistantServiceUpdateDialog } from "@/components/assistant/assistant-service-update-dialog";
import type { ActionCardParams } from "@/schemas/assistant-actions";

export type DialogVariant = Exclude<
  ActionCardParams["variant"],
  "catalog" | "custom" | "service_reauthorize" | "unknown"
>;
type ParamsOf<V extends DialogVariant> = Extract<
  ActionCardParams,
  { variant: V }
>;
export type DialogParams = ParamsOf<DialogVariant>;

interface AssistantDialogProps<P> {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: P;
  readonly onComplete: (completion: unknown) => void;
}

export interface DialogBinding<V extends DialogVariant> {
  readonly Dialog: ComponentType<AssistantDialogProps<unknown>>;
  readonly toProps: (params: ParamsOf<V>) => unknown;
}

export const ACTION_DIALOGS: {
  readonly [V in DialogVariant]: DialogBinding<V>;
} = {
  key_create: {
    Dialog: AssistantKeyCreateDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      name: params.name,
      platform: params.platform,
      allowedServiceIds: params.allowed_service_ids,
    }),
  },
  key_rotate: {
    Dialog: AssistantKeyRotateDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({ keyId: params.key_id }),
  },
  key_update: {
    Dialog: AssistantKeyUpdateDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      keyId: params.key_id,
      name: params.name,
      platform: params.platform,
      description: params.description,
    }),
  },
  key_delete: {
    Dialog: AssistantKeyDeleteDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({ keyId: params.key_id }),
  },
  key_extend_scope: {
    Dialog: AssistantKeyScopeDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      keyId: params.key_id,
      addServiceIds: params.add_service_ids,
    }),
  },
  key_bind_credential: {
    Dialog: AssistantKeyBindDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      keyId: params.key_id,
      userServiceId: params.user_service_id,
      externalKeyId: params.external_key_id,
    }),
  },
  service_update: {
    Dialog: AssistantServiceUpdateDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      userServiceId: params.user_service_id,
      name: params.name,
      endpointUrl: params.endpoint_url,
      authMethod: params.auth_method,
      authKeyName: params.auth_key_name,
    }),
  },
  service_delete: {
    Dialog: AssistantServiceDeleteDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({ userServiceId: params.user_service_id }),
  },
  service_route: {
    Dialog: AssistantServiceRouteDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({
      userServiceId: params.user_service_id,
      viaNodeId: params.via_node_id,
    }),
  },
  service_rotate_credential: {
    Dialog: AssistantServiceRotateCredentialDialog as ComponentType<
      AssistantDialogProps<unknown>
    >,
    toProps: (params) => ({ userServiceId: params.user_service_id }),
  },
};

export function isDialogVariant(
  variant: ActionCardParams["variant"],
): variant is DialogVariant {
  return Object.prototype.hasOwnProperty.call(ACTION_DIALOGS, variant);
}

export function isDialogParams(
  params: ActionCardParams,
): params is DialogParams {
  return isDialogVariant(params.variant);
}

export function dialogBindingFor<V extends DialogVariant>(
  variant: V,
): DialogBinding<V> {
  return ACTION_DIALOGS[variant];
}
