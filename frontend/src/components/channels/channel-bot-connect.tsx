import { useState, type ComponentType, type ReactNode } from "react";
import type { UseFormReturn } from "react-hook-form";
import { useManagedOnboarding } from "@/hooks/use-channel-managed";
import { CHANNEL_PLATFORMS } from "@/lib/channel-platforms";
import type { CreateChannelBotFormData } from "@/schemas/channels";
import type { ChannelPlatform } from "@/types/channels";
import { MANAGED_FLOW_COMPONENTS } from "./managed-flows";
import { TelegramNewConnect } from "./telegram-new-connect";

export interface ChannelBotConnectProps {
  readonly platform: ChannelPlatform;
  readonly label: string;
  readonly orgId: string | null;
  readonly form: UseFormReturn<CreateChannelBotFormData>;
  readonly onConnected: (id: string, replace?: boolean) => void;
  readonly renderFields: (state: {
    disabled: boolean;
    scopeDescription?: string;
  }) => ReactNode;
  readonly children: ReactNode;
}

const CONNECT_COMPONENTS: Partial<
  Record<ChannelPlatform, ComponentType<ChannelBotConnectProps>>
> = {
  "telegram-new": TelegramNewConnect,
};

export function ChannelBotConnect(props: ChannelBotConnectProps) {
  const Connect = CONNECT_COMPONENTS[props.platform] ?? ManagedBotConnect;
  return <Connect key={props.platform} {...props} />;
}

function ManagedBotConnect({
  platform,
  label,
  orgId,
  onConnected,
  renderFields,
  children,
}: ChannelBotConnectProps) {
  const descriptor = CHANNEL_PLATFORMS[platform];
  const managed = useManagedOnboarding(
    platform,
    Boolean(descriptor.managedFlow),
  );
  const [advanced, setAdvanced] = useState(false);
  const available = Boolean(descriptor.managedFlow && managed.data?.available);
  const Connect = descriptor.managedFlow
    ? MANAGED_FLOW_COMPONENTS[descriptor.managedFlow].Connect
    : undefined;
  return (
    <>
      {renderFields({ disabled: false })}
      {descriptor.managedOnly && !available && (
        <p role="status" className="text-xs text-muted-foreground">
          {managed.isLoading
            ? "Loading account connection..."
            : managed.isError
              ? "Unable to load account connection settings. Retry shortly."
              : `Not available until an admin configures ${descriptor.label}.`}
        </p>
      )}
      {available && managed.data && Connect && (
        <>
          <Connect
            key={`${platform}:${orgId ?? "personal"}`}
            platform={platform}
            bootstrap={managed.data}
            label={label}
            orgId={orgId}
            onConnected={(bot) => onConnected(bot.id)}
          />
          {!descriptor.managedOnly && (
            <details
              open={advanced}
              onToggle={(event) => setAdvanced(event.currentTarget.open)}
              className="border-t border-border pt-4"
            >
              <summary className="cursor-pointer text-xs text-muted-foreground">
                {descriptor.advancedLabel}
              </summary>
            </details>
          )}
        </>
      )}
      {!descriptor.managedOnly && (!available || advanced) && children}
    </>
  );
}
