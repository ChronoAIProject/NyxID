import type { ChannelBotDetail, ChannelPlatform, ChannelPlatformDescriptor, ChannelRegistrationField, CreateChannelBotRequest } from "@/types/channels";

export const TELEGRAM_MANAGER_DELETION_NOTE = "Bot creation and the manager webhook will remain available. The manager token stays in Admin → Platform Credentials → Telegram — bot creation.";

export type ChannelCredentialField = Exclude<Extract<keyof CreateChannelBotRequest, string>, "platform" | "label" | "target_org_id">;
export type ManagedFlow = "meta_embedded_signup" | "oauth_connection";
export interface ChannelFieldDescriptor extends Omit<ChannelRegistrationField, "name"> {
  readonly name: ChannelCredentialField;
  readonly configuredKey?: keyof Pick<ChannelBotDetail, "app_secret_configured" | "lark_verification_token_configured" | "lark_encrypt_key_configured">;
}
const configuredKeys: Record<string, ChannelFieldDescriptor["configuredKey"]> = {
  app_secret_encrypted: "app_secret_configured",
  lark_verification_token_encrypted: "lark_verification_token_configured",
  lark_encrypt_key_encrypted: "lark_encrypt_key_configured",
};

export function platformView(descriptor?: ChannelPlatformDescriptor, id = "") {
  const candidate = descriptor?.managed_onboarding?.flow;
  const flow: ManagedFlow | undefined = candidate === "meta_embedded_signup" || candidate === "oauth_connection" ? candidate : undefined;
  return {
    label: descriptor?.display_name ?? id,
    activities: descriptor?.activities ?? [],
    enabled: descriptor?.enabled ?? false,
    fields: (descriptor?.registration.fields ?? []).map((field): ChannelFieldDescriptor => ({
      ...field, name: field.name as ChannelCredentialField, configuredKey: configuredKeys[field.storage],
    })),
    managedOnly: descriptor?.managed_only ?? false,
    managedFlow: flow,
    webhookDocs: descriptor?.registration.documentation_url ?? undefined,
    webhookIngestion: descriptor?.registration.webhook_ingestion ?? false,
    identityLabel: descriptor?.registration.fields.find((field) => field.storage === "platform_bot_id")?.label,
    detailFields: (descriptor?.registration.fields ?? []).filter((f) => !f.secret && f.storage !== "platform_bot_id")
      .map((f) => ({ name: f.name as keyof ChannelBotDetail, label: f.label })),
    setupNote: descriptor?.registration.setup_instructions.length ? {
      title: "Setup instructions", text: descriptor.registration.setup_instructions.join(" "),
    } : undefined,
    advancedLabel: "Advanced: use your own credentials",
    connectLabel: `Connect ${descriptor?.display_name ?? id} account`,
    connectedLabel: "Connected account",
    deletionNote: descriptor?.managed_onboarding ? "The platform account stays connected. Manage it separately in your account's connections." : undefined,
  };
}
export function channelPlatformViews(descriptors: readonly ChannelPlatformDescriptor[]) {
  return Object.fromEntries(descriptors.map((d) => [d.platform, platformView(d)]));
}

/** Search parsing is syntactic. The live catalog decides whether a flow is enabled. */
export function managedConnectPlatform(value: unknown): ChannelPlatform | undefined {
  return typeof value === "string" && /^[a-z][a-z0-9-]{0,63}$/.test(value) ? value as ChannelPlatform : undefined;
}

export function channelBotRegistrationPayload(data: CreateChannelBotRequest, descriptor: ChannelPlatformDescriptor): CreateChannelBotRequest {
  const fields = Object.fromEntries(descriptor.registration.fields.flatMap(({ name }) => {
    const value = data[name as ChannelCredentialField]?.trim();
    return value ? [[name, value]] : [];
  }));
  return { platform: data.platform, label: data.label.trim(), target_org_id: data.target_org_id || undefined, ...fields };
}
export type ChannelMutableField = Exclude<ChannelCredentialField, "phone_number_id" | "waba_id" | "public_key">;
export function editableChannelFields(descriptor: ReturnType<typeof platformView>) {
  return descriptor.fields.filter((field): field is ChannelFieldDescriptor & { readonly name: ChannelMutableField } => field.patchable);
}
