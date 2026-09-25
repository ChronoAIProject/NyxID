import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { ArrowRight, Check, ExternalLink } from "lucide-react";
import { toast } from "sonner";
import { useChannelPlatformViews } from "@/hooks/use-channel-platforms";
import { useOrgs } from "@/hooks/use-orgs";
import { useCreateChannelBot } from "@/hooks/use-channel-bots";
import {
  buildCreateChannelBotSchema,
  type CreateChannelBotFormData,
} from "@/schemas/channels";
import { ApiError } from "@/lib/api-client";
import {
  channelBotRegistrationPayload,
  managedConnectPlatform,
} from "@/lib/channel-platforms";
import { CopyableUrlCallout } from "@/components/shared/copyable-url-callout";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  ChannelPlatform,
  CreateChannelBotResponse,
} from "@/types/channels";
import { ChannelBotConnect } from "./channel-bot-connect";

const EMPTY_BOT_CREDENTIALS = {
  bot_token: "",
  app_id: "",
  app_secret: "",
  verification_token: "",
  encrypt_key: "",
  public_key: "",
  phone_number_id: "",
  waba_id: "",
};

export function ChannelBotSetup({
  open,
  onOpenChange,
  defaultOrgId,
  defaultPlatform,
  defaultLabel = "",
  fullPage = false,
  prefill,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** Pre-select this org in the scope picker when the page already has one
   *  active. `null` defaults to personal. */
  readonly defaultOrgId: string | null;
  readonly defaultPlatform?: ChannelPlatform;
  readonly defaultLabel?: string;
  readonly fullPage?: boolean;
  readonly prefill?: Readonly<Record<string, string>>;
  readonly onComplete?: () => void;
}) {
  const navigate = useNavigate();
  const catalog = useChannelPlatformViews();
  const { getPlatform } = catalog;
  const createBot = useCreateChannelBot();
  const orgs = useOrgs();
  const [createdBot, setCreatedBot] = useState<CreateChannelBotResponse | null>(
    null,
  );
  const [connectedBotId, setConnectedBotId] = useState<string | null>(null);
  const completedId = createdBot?.id ?? connectedBotId;
  const dialogContentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (createdBot && dialogContentRef.current)
      dialogContentRef.current.scrollTop = 0;
  }, [createdBot]);
  const form = useAppForm<CreateChannelBotFormData>({
    mode: "onChange",
    resolver: zodResolver(
      buildCreateChannelBotSchema(catalog.data?.platforms ?? []),
    ),
    defaultValues: {
      ...EMPTY_BOT_CREDENTIALS,
      platform: defaultPlatform ?? "telegram",
      label: defaultLabel,
      target_org_id: defaultOrgId ?? undefined,
    },
  });

  const {
    register,
    handleSubmit,
    setValue,
    control,
    reset,
    getValues,
    formState: { errors, isDirty, isValid },
  } = form;

  const platform = useWatch({ control, name: "platform" });
  const setupNote = getPlatform(platform).setupNote;
  const targetOrgId = useWatch({ control, name: "target_org_id" }) ?? null;
  const showScope =
    !fullPage ||
    Boolean(targetOrgId) ||
    orgs.isError ||
    orgs.data?.some((org) => org.your_role === "admin");
  const label = useWatch({ control, name: "label" });
  const previousPlatform = useRef(defaultPlatform);

  useEffect(() => {
    for (const [name, value] of Object.entries(prefill ?? {})) {
      setValue(name, value, {
        shouldDirty: true,
        shouldTouch: false,
        shouldValidate: true,
      });
    }
  }, [prefill, setValue]);

  // Opening remounts the form with the current page scope. A new deep link
  // can also select a flow while this dialog is already open.
  useEffect(() => {
    const changed = previousPlatform.current !== defaultPlatform;
    previousPlatform.current = defaultPlatform;
    if (
      !changed ||
      !defaultPlatform ||
      defaultPlatform === getValues("platform")
    )
      return;
    reset({
      ...EMPTY_BOT_CREDENTIALS,
      platform: defaultPlatform,
      label: defaultLabel,
      target_org_id: defaultOrgId ?? undefined,
    });
  }, [defaultPlatform, defaultLabel, defaultOrgId, getValues, reset]);

  function changePlatform(next: ChannelPlatform) {
    reset(
      {
        ...EMPTY_BOT_CREDENTIALS,
        platform: next,
        label,
        target_org_id: targetOrgId ?? undefined,
      },
      { keepDefaultValues: true },
    );
    void navigate({
      to: "/channel-bots",
      search: {
        connect: managedConnectPlatform(next),
        label,
        target_org_id: targetOrgId ?? undefined,
      },
      replace: true,
    });
  }

  function onSubmit(data: CreateChannelBotFormData) {
    if (getPlatform(data.platform).fields.length === 0) return;
    const descriptor = catalog.data?.platforms.find(
      (p) => p.platform === data.platform && p.enabled,
    );
    if (!descriptor) return;
    const payload = channelBotRegistrationPayload(data, descriptor);
    createBot.mutate(payload, {
      onSuccess: (result) => {
        if (result.webhook_secret || fullPage) {
          setCreatedBot(result);
          if (fullPage) onComplete?.();
          if (!fullPage) reset();
          createBot.reset();
          return;
        }
        toast.success(
          result.credential_source === "telegram_manager"
            ? `Telegram manager "${result.platform_bot_username}" connected`
            : `Bot "${result.platform_bot_username}" created successfully`,
        );
        reset();
        createBot.reset();
        if (!fullPage) onOpenChange(false);
        void navigate({
          to: "/channel-bots/$botId",
          params: { botId: result.id },
        });
      },
      onError: (err) => {
        const message =
          err instanceof ApiError ? err.message : "Failed to create bot";
        toast.error(message);
      },
    });
  }

  function changeOpen(next: boolean) {
    onOpenChange(next);
    if (!next) {
      const id = createdBot?.id;
      setCreatedBot(null);
      createBot.reset();
      reset();
      if (id)
        void navigate({ to: "/channel-bots/$botId", params: { botId: id } });
    }
  }

  const content = (
    <>
      {!fullPage && (
        <DialogHeader>
          <DialogTitle>
            {createdBot
              ? `${getPlatform(createdBot.platform).label} Bot Created`
              : "Add Channel Bot"}
          </DialogTitle>
          <DialogDescription>
            {createdBot
              ? "Store this verification secret now. It will not be shown again."
              : "Connect a messaging platform bot to your AI agents."}
          </DialogDescription>
        </DialogHeader>
      )}

      {catalog.isLoading && (
        <p role="status" className="text-xs text-muted-foreground">
          Loading platforms...
        </p>
      )}
      {catalog.isError && (
        <p role="alert" className="text-xs text-destructive">
          Unable to load platforms.{" "}
          <button type="button" onClick={() => void catalog.refetch()}>
            Retry
          </button>
        </p>
      )}
      {completedId ? (
        <div className="space-y-4">
          {fullPage && (
            <div
              role="status"
              className="flex items-start gap-3 rounded-lg border border-success/20 bg-success/5 p-4"
            >
              <Check
                className="mt-0.5 size-4 shrink-0 text-success"
                aria-hidden="true"
              />
              <div className="space-y-1">
                <p className="text-[13px] font-medium">
                  {label || "Channel bot created"}
                </p>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {createdBot?.webhook_secret
                    ? "Save the verification secret below. It will not be shown again."
                    : "Your bot has been added to NyxID. Open it to review setup and assign an agent."}
                </p>
              </div>
            </div>
          )}
          {createdBot?.webhook_url && (
            <CopyableUrlCallout
              label="Callback URL"
              url={createdBot.webhook_url}
              docsHref={getPlatform(createdBot.platform).webhookDocs}
            />
          )}
          {createdBot?.webhook_secret && (
            <CopyableUrlCallout
              label={createdBot.webhook_secret_label ?? "Webhook Secret"}
              url={createdBot.webhook_secret}
            />
          )}
          {Boolean(createdBot?.setup_instructions?.length) && (
            <ol className="list-decimal space-y-2 pl-4 text-xs text-muted-foreground">
              {createdBot?.setup_instructions?.map((instruction) => (
                <li key={instruction}>{instruction}</li>
              ))}
            </ol>
          )}
          <div className="flex justify-end">
            <Button
              variant="primary"
              className={fullPage ? "w-full" : undefined}
              onClick={() => {
                if (!fullPage) onOpenChange(false);
                void navigate({
                  to: "/channel-bots/$botId",
                  params: { botId: completedId },
                });
              }}
            >
              {fullPage ? "Open channel bot" : "Done"}
              {fullPage && <ArrowRight aria-hidden="true" />}
            </Button>
          </div>
        </div>
      ) : (
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <ChannelBotConnect
            platform={platform}
            label={label}
            orgId={targetOrgId}
            form={form}
            fullPage={fullPage}
            preferManual={Boolean(prefill && Object.keys(prefill).length)}
            onConnected={(id, replace) => {
              if (fullPage) {
                setConnectedBotId(id);
                onComplete?.();
                return;
              }
              if (!fullPage) onOpenChange(false);
              void navigate({
                to: "/channel-bots/$botId",
                params: { botId: id },
                replace,
              });
            }}
            renderFields={({ disabled, scopeDescription }) => (
              <>
                {showScope && (
                  <div className="space-y-2">
                    <Label htmlFor="scope">
                      {fullPage ? "Create for" : "Scope"}
                    </Label>
                    <OrgScopeSelect
                      id="scope"
                      aria-describedby="scope-description"
                      value={targetOrgId}
                      disabled={disabled}
                      onChange={(next) =>
                        setValue("target_org_id", next ?? undefined)
                      }
                      label={fullPage ? "Create for" : "Scope"}
                    />
                    <p
                      id="scope-description"
                      className="text-xs text-muted-foreground"
                    >
                      {scopeDescription ??
                        (fullPage
                          ? "Choose your personal account or an organization."
                          : "Choose where this bot lives. Org bots are visible to every org admin and can be bound to org-owned agent keys.")}
                    </p>
                  </div>
                )}

                {!fullPage && (
                  <div className="space-y-2">
                    <Label htmlFor="platform">Platform</Label>
                    <Select
                      value={platform}
                      onValueChange={(value) =>
                        changePlatform(value as ChannelPlatform)
                      }
                    >
                      <SelectTrigger id="platform">
                        <SelectValue placeholder="Select platform" />
                      </SelectTrigger>
                      <SelectContent>
                        {Object.entries(catalog.platforms)
                          .filter(([, descriptor]) => descriptor.enabled)
                          .map(([id, descriptor]) => (
                            <SelectItem key={id} value={id}>
                              {descriptor.label}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                    {errors.platform && (
                      <p className="text-xs text-destructive">
                        {errors.platform.message}
                      </p>
                    )}
                    {getPlatform(platform).enabled && (
                      <Link
                        to="/channel-bots/connect/$platform"
                        params={{ platform }}
                        search={{
                          label: label || undefined,
                          target_org_id: targetOrgId ?? undefined,
                        }}
                        className="inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
                      >
                        <ExternalLink className="size-3" aria-hidden="true" />
                        Open full setup page
                      </Link>
                    )}
                  </div>
                )}

                <div className="space-y-2">
                  <Label htmlFor="label">
                    {fullPage ? "Bot name" : "Label"}
                  </Label>
                  <Input
                    id="label"
                    disabled={disabled}
                    placeholder={`My ${getPlatform(platform).label} Bot`}
                    {...register("label")}
                  />
                  {errors.label && (
                    <p className="text-xs text-destructive">
                      {errors.label.message}
                    </p>
                  )}
                </div>
              </>
            )}
          >
            {platform === "telegram" && !fullPage && (
              <p className="text-xs text-muted-foreground">
                To connect an existing Telegram manager, enter its bot token
                here. NyxID keeps its bot creation webhook and adds conversation
                routing. You can then choose a public default agent or an exact
                chat route.
              </p>
            )}
            {setupNote && (
              <details
                className="rounded-lg border border-border/70 bg-muted/30 p-3"
                open={!fullPage}
              >
                <summary className="cursor-pointer text-[12px] font-medium">
                  {setupNote.title}
                </summary>
                <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                  {setupNote.text}
                </p>
              </details>
            )}
            {getPlatform(platform).fields.map((field) => (
              <div key={field.name} className="space-y-2">
                <Label htmlFor={field.name}>
                  {field.label}
                  {field.required ? "" : " (optional)"}
                </Label>
                <Input
                  id={field.name}
                  type={field.secret ? "password" : "text"}
                  autoComplete={field.secret ? "new-password" : "off"}
                  {...register(field.name)}
                />
                {field.hint && (
                  <p className="text-xs text-muted-foreground">{field.hint}</p>
                )}
                {errors[field.name] && (
                  <p className="text-xs text-destructive">
                    {errors[field.name]?.message}
                  </p>
                )}
              </div>
            ))}

            <div
              className={
                fullPage ? "flex flex-col pt-2" : "flex justify-end gap-2"
              }
            >
              {!fullPage && (
                <Button
                  type="button"
                  variant="outline"
                  disabled={createBot.isPending}
                  onClick={() => onOpenChange(false)}
                >
                  Cancel
                </Button>
              )}
              <Button
                variant="primary"
                type="submit"
                isLoading={createBot.isPending}
                disabled={!catalog.data || !isDirty || !isValid}
              >
                {fullPage
                  ? platform === "telegram"
                    ? "Connect bot"
                    : "Create channel bot"
                  : "Add Bot"}
                {fullPage && <ArrowRight aria-hidden="true" />}
              </Button>
            </div>
          </ChannelBotConnect>
        </form>
      )}
    </>
  );

  if (fullPage)
    return (
      <div ref={dialogContentRef} className="space-y-4">
        {content}
      </div>
    );

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        ref={dialogContentRef}
        className="channel-connect-dialog max-h-[90dvh] overflow-y-auto md:max-w-md"
      >
        {content}
      </DialogContent>
    </Dialog>
  );
}
