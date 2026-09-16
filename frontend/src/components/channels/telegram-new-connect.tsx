import { useEffect } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useTelegramNewConfiguration } from "@/hooks/use-telegram-new";
import { useAuthStore } from "@/stores/auth-store";
import type { ChannelBotConnectProps } from "./channel-bot-connect";
import { TelegramNew } from "./telegram-new";

export function TelegramNewConnect({
  label,
  orgId,
  form,
  onConnected,
  renderFields,
}: ChannelBotConnectProps) {
  const navigate = useNavigate();
  const search = useSearch({ strict: false });
  const actor = useAuthStore((state) => state.user?.id);
  const configuration = useTelegramNewConfiguration(search.request_id);
  const saved = configuration.data?.request;
  const request =
    saved && !["cancelled", "expired"].includes(saved.status)
      ? saved
      : undefined;
  const savedOrgId =
    request && request.owner_user_id !== actor
      ? request.owner_user_id
      : undefined;
  const { setValue } = form;

  useEffect(() => {
    if (!request) return;
    const options = { shouldDirty: false, shouldTouch: false };
    setValue("label", request.label, options);
    setValue("target_org_id", savedOrgId, options);
  }, [request, savedOrgId, setValue]);

  useEffect(() => {
    if (request?.status === "connected") return;
    const nextLabel = request?.label ?? label;
    const nextOrgId = request ? savedOrgId : (orgId ?? undefined);
    const requestId = request?.id ?? search.request_id;
    if (
      search.connect === "telegram-new" &&
      search.label === nextLabel &&
      search.target_org_id === nextOrgId &&
      search.request_id === requestId
    )
      return;
    void navigate({
      to: "/channel-bots",
      search: {
        connect: "telegram-new",
        label: nextLabel,
        target_org_id: nextOrgId,
        request_id: requestId,
      },
      replace: true,
    });
  }, [
    request,
    savedOrgId,
    label,
    orgId,
    search.connect,
    search.label,
    search.target_org_id,
    search.request_id,
    navigate,
  ]);

  async function saveRequest(requestId?: string) {
    await navigate({
      to: "/channel-bots",
      search: {
        connect: "telegram-new",
        label,
        target_org_id: orgId ?? undefined,
        request_id: requestId,
      },
      replace: true,
    });
  }

  return (
    <>
      {renderFields({
        disabled: Boolean(request) || configuration.isPending,
        scopeDescription: request
          ? request.status === "provisioning" || request.status === "connected"
            ? "These details are saved. You can manage the bot once it is connected."
            : "These details are saved. To change them, choose Cancel setup below."
          : undefined,
      })}
      <TelegramNew
        key={actor}
        label={label}
        orgId={orgId}
        requestId={search.request_id}
        onStarted={saveRequest}
        onCancelled={() => saveRequest()}
        onConnected={(id) => onConnected(id, true)}
      />
    </>
  );
}
