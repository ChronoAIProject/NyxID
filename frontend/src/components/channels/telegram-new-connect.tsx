import { useChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";
import { useCallback, useEffect, useState } from "react";
import { useIsMutating } from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useTelegramNewConfiguration } from "@/hooks/use-telegram-new";
import { useAuthStore } from "@/stores/auth-store";
import type { ChannelBotConnectProps } from "./channel-bot-connect";
import { TelegramNew } from "./telegram-new";

export function TelegramNewConnect({
  label,
  orgId,
  form,
  fullPage = false,
  stayInPlace = false,
  returnTo,
  onConnected,
  renderFields,
}: ChannelBotConnectProps) {
  const navigate = useNavigate();
  const link = useChannelConnectLinkContext();
  const search = useSearch({ strict: false });
  const actor = useAuthStore((state) => state.user?.id);
  const [localRequest, setLocalRequest] = useState<{ actor: string | undefined; id: string | undefined }>();
  const localRequestId = localRequest && localRequest.actor === actor ? localRequest.id : undefined;
  const requestId = localRequestId ?? (stayInPlace ? undefined : link ? link.telegramRequestId : search.request_id);
  const configuration = useTelegramNewConfiguration(requestId, false);
  const pending = useIsMutating({ mutationKey: ["telegram-new", actor] }) > 0;
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
  const setupSearch = useCallback(
    (next: { label: string; target_org_id?: string; request_id?: string }) =>
      returnTo ? { ...next, return_to: returnTo } : next,
    [returnTo],
  );
  const updateSearch = useCallback(
    (next: { label: string; target_org_id?: string; request_id?: string }) =>
      link || stayInPlace ? (link ? link.refresh() : Promise.resolve()) : fullPage
        ? navigate({
            to: "/channel-bots/connect/$platform",
            params: { platform: "telegram-new" },
            search: setupSearch(next),
            replace: true,
          })
        : navigate({
            to: "/channel-bots",
            search: { connect: "telegram-new", ...next },
            replace: true,
          }),
    [fullPage, navigate, link, setupSearch, stayInPlace],
  );

  if (stayInPlace && request && !localRequestId) {
    setLocalRequest({ actor, id: request.id });
  }

  useEffect(() => {
    if (!request) return;
    const options = { shouldDirty: false, shouldTouch: false };
    setValue("label", request.label, options);
    setValue("target_org_id", savedOrgId, options);
  }, [request, savedOrgId, setValue]);

  useEffect(() => {
    if (link || stayInPlace || request?.status === "connected") return;
    const nextLabel = request?.label ?? label;
    const nextOrgId = request ? savedOrgId : (orgId ?? undefined);
    const requestId = request?.id ?? search.request_id;
    if (
      (fullPage || search.connect === "telegram-new") &&
      search.label === nextLabel &&
      search.target_org_id === nextOrgId &&
      search.request_id === requestId
    )
      return;
    void updateSearch({
      label: nextLabel,
      target_org_id: nextOrgId,
      request_id: requestId,
    });
  }, [
    request,
    link,
    savedOrgId,
    label,
    orgId,
    search.connect,
    search.label,
    search.target_org_id,
    search.request_id,
    fullPage,
    stayInPlace,
    updateSearch,
  ]);

  async function saveRequest(requestId?: string) {
    setLocalRequest({ actor, id: requestId });
    if (stayInPlace) return;
    await updateSearch({
      label,
      target_org_id: orgId ?? undefined,
      request_id: requestId,
    });
  }

  return (
    <>
      {renderFields({
        disabled: Boolean(request) || configuration.isPending || pending,
        scopeDescription: request
          ? fullPage || request.status === "provisioning" || request.status === "connected"
            ? "These details are saved. You can manage the bot once it is connected."
            : "These details are saved. To change them, choose Cancel setup below."
          : undefined,
      })}
      <TelegramNew
        key={actor}
        fullPage={fullPage}
        stayInPlace={stayInPlace}
        label={label}
        orgId={orgId}
        requestId={requestId}
        onStarted={saveRequest}
        onCancelled={() => saveRequest()}
        onConnected={(id, username) => onConnected(id, true, undefined, username ? `@${username}` : label)}
      />
    </>
  );
}
