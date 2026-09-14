import { useEffect } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { ArrowLeft } from "lucide-react";
import { useTelegramNewConfiguration } from "@/hooks/use-telegram-new";
import { useAuthStore } from "@/stores/auth-store";
import { telegramNewBeginSchema } from "@/schemas/telegram-new";
import { TelegramNew } from "@/components/channels/telegram-new";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAppForm } from "@/components/ui/form";

export function TelegramSetupPage() {
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as {
    label?: string;
    target_org_id?: string;
  };
  const actor = useAuthStore((state) => state.user?.id);
  const configuration = useTelegramNewConfiguration();
  const request = configuration.data?.request;
  const savedOrgId =
    request && request.owner_user_id !== actor
      ? request.owner_user_id
      : undefined;
  const { register, control, setValue } = useAppForm<{
    label: string;
    target_org_id?: string;
  }>({
    mode: "onChange",
    resolver: zodResolver(telegramNewBeginSchema),
    values: {
      label: request?.label ?? search.label ?? "",
      target_org_id: request ? savedOrgId : search.target_org_id,
    },
  });
  const label = useWatch({ control, name: "label" });
  const orgId = useWatch({ control, name: "target_org_id" });

  useEffect(() => {
    if (
      !request ||
      (search.label === request.label && search.target_org_id === savedOrgId)
    )
      return;
    void navigate({
      to: "/channel-bots",
      search: {
        connect: "telegram-new",
        label: request.label,
        target_org_id: savedOrgId,
      },
      replace: true,
    });
  }, [request, savedOrgId, search.label, search.target_org_id, navigate]);

  function saveDraft(nextLabel: string, nextOrgId: string | undefined) {
    void navigate({
      to: "/channel-bots",
      search: {
        connect: "telegram-new",
        label: nextLabel,
        target_org_id: nextOrgId,
      },
      replace: true,
    });
  }

  return (
    <section
      aria-label="Telegram bot setup"
      className="mx-auto w-full max-w-2xl space-y-6 pb-6"
    >
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void navigate({ to: "/channel-bots", search: {} })}
      >
        <ArrowLeft className="size-4" />
        Back to Channel Bots
      </Button>
      <PageHeader
        title="Create a Telegram bot"
        description="Create a bot in Telegram, then connect it to an AI agent in NyxID."
      />
      <div className="space-y-4 rounded-xl border border-border bg-card p-4 sm:p-5">
        <div className="space-y-2">
          <Label htmlFor="telegram-label">Bot label in NyxID</Label>
          <Input
            id="telegram-label"
            placeholder="e.g. Support assistant"
            maxLength={128}
            disabled={Boolean(request) || configuration.isPending}
            {...register("label", {
              onChange: (event: React.ChangeEvent<HTMLInputElement>) =>
                saveDraft(event.target.value, orgId),
            })}
          />
          <p className="text-xs text-muted-foreground">
            This label helps you find the bot in NyxID. You will choose the name
            people see in Telegram in the next step.
          </p>
        </div>
        <div className="space-y-2">
          <Label>Connect to</Label>
          <OrgScopeSelect
            label="Connect to"
            value={orgId ?? null}
            disabled={Boolean(request) || configuration.isPending}
            onChange={(next) => {
              setValue("target_org_id", next ?? undefined);
              saveDraft(label, next ?? undefined);
            }}
          />
          <p className="text-xs text-muted-foreground">
            {request
              ? "These details are saved. To change them, choose Cancel setup below."
              : "Choose where the bot will appear in NyxID: your personal account or an organization you manage."}
          </p>
        </div>
      </div>
      <TelegramNew
        key={actor}
        label={label}
        orgId={orgId ?? null}
        onConnected={(id) => {
          void navigate({
            to: "/channel-bots/$botId",
            params: { botId: id },
            replace: true,
          });
        }}
      />
    </section>
  );
}
