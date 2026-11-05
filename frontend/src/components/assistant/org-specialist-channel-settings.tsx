import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { CopyableField } from "@/components/shared/copyable-field";
import { useOrgSpecialistChannel } from "@/hooks/use-org-specialist-channel";
import {
  orgChannelGuestFormSchema,
  type OrgChannelGuestForm,
  type OrgChannelSettings,
} from "@/schemas/org-specialist-channel";

export function OrgSpecialistChannelSettings({ id }: { readonly id: string }) {
  const [challenge, setChallenge] = useState<string>();
  const api = useOrgSpecialistChannel(id, challenge);
  const error =
    api.settings.error ??
    api.save.error ??
    api.issue.error ??
    api.confirm.error ??
    api.revoke.error ??
    api.status.error;
  return (
    <div className="space-y-2 border-t border-border pt-2 text-12">
      <p className="text-muted-foreground">
        Verify your own chat account to use your organization membership. Group
        replies are public; your private conversation stays private.
      </p>
      {error ? (
        <p role="alert" className="text-destructive">
          {error.message}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={api.issue.isPending}
          onClick={() => {
            api.confirm.reset();
            api.revoke.reset();
            void api.issue
              .mutateAsync()
              .then((value) => setChallenge(value.id))
              .catch(() => {});
          }}
        >
          Verify my chat account
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={api.revoke.isPending}
          onClick={() => api.revoke.mutate()}
        >
          Unlink my chat accounts
        </Button>
      </div>
      {api.issue.data?.code && challenge && !api.confirm.isSuccess ? (
        <div className="space-y-2">
          <p>
            Send this code in a private message to the bot, then confirm the
            account here. Never send it to a group.
          </p>
          <CopyableField
            value={api.issue.data.code}
            label="Member verification code"
          />
          {api.status.data?.state === "observed" ? (
            <>
              <p>
                Confirm that “{api.status.data.sender_label ?? "Chat account"}”
                is your account. Its messages use your member permissions and
                billing.
              </p>
              <Button
                size="sm"
                variant="primary"
                disabled={api.confirm.isPending}
                onClick={() => api.confirm.mutate()}
              >
                Confirm my account
              </Button>
            </>
          ) : (
            <p className="text-text-tertiary">
              Waiting for your message. The code expires after 10 minutes.
            </p>
          )}
        </div>
      ) : null}
      {api.confirm.isSuccess ? (
        <p role="status">Your chat account is verified.</p>
      ) : null}
      {api.revoke.isSuccess ? (
        <p role="status">Your chat accounts are unlinked.</p>
      ) : null}
      {api.settings.data ? (
        <>
          <p>
            Guests: {api.settings.data.guest_enabled ? "Enabled" : "Off"}.
            Organization spend today: {api.settings.data.spent_today_credits}{" "}
            credits ({api.settings.data.reserved_today_credits} reserved).
          </p>
          {api.settings.data.can_manage ? (
            <GuestSettings
              key={JSON.stringify(api.settings.data)}
              settings={api.settings.data}
              saving={api.save.isPending}
              save={(value) => api.save.mutate(value)}
            />
          ) : (
            <p className="text-text-tertiary">
              Only organization admins can change guest access.
            </p>
          )}
        </>
      ) : null}
    </div>
  );
}

function GuestSettings({
  settings,
  saving,
  save,
}: {
  readonly settings: OrgChannelSettings;
  readonly saving: boolean;
  readonly save: (value: OrgChannelGuestForm) => void;
}) {
  const form = useAppForm<OrgChannelGuestForm>({
    resolver: zodResolver(orgChannelGuestFormSchema),
    defaultValues: {
      enabled: settings.guest_enabled,
      service_ids: settings.delegated_service_ids,
      daily_credits: settings.daily_credits ?? settings.default_daily_credits,
    },
  });
  const selected = form.watch("service_ids");
  return (
    <form onSubmit={form.handleSubmit(save)} className="space-y-2">
      <p className="text-muted-foreground">
        The organization pays for every guest model and service call, including
        platform keys. Delegate only services approved for public visitors. With
        no services selected, guests can chat with the model only.
      </p>
      {settings.readiness_reason ? (
        <p role="status" className="text-destructive">
          {settings.readiness_reason === "org_wallet_not_provisioned"
            ? "Provision the organization billing wallet and subscription before enabling guests."
            : "The organization wallet is suspended."}{" "}
          <a href={`/billing?ownerId=${encodeURIComponent(settings.org_id)}`} className="type-button underline">Open organization Billing</a>
        </p>
      ) : null}
      <Label className="flex items-center gap-2">
        <Checkbox
          checked={form.watch("enabled")}
          disabled={
            saving ||
            (Boolean(settings.readiness_reason) && !form.watch("enabled"))
          }
          onCheckedChange={(value) => form.setValue("enabled", value === true)}
        />
        Enable guests for this binding
      </Label>
      <Label className="block space-y-1">
        <span>Daily guest budget (credits, UTC)</span>
        <Input
          {...form.register("daily_credits")}
          inputMode="decimal"
          disabled={saving}
        />
      </Label>
      {form.formState.errors.daily_credits ? (
        <p role="alert" className="text-destructive">
          {form.formState.errors.daily_credits.message}
        </p>
      ) : null}
      <fieldset className="space-y-1">
        <Label asChild><legend>Delegated services</legend></Label>
        {settings.available_services.map((service) => (
          <Label key={service.id} className="flex items-center gap-2">
            <Checkbox
              disabled={saving}
              checked={selected.includes(service.id)}
              onCheckedChange={(value) =>
                form.setValue(
                  "service_ids",
                  value === true
                    ? [...selected, service.id]
                    : selected.filter((id) => id !== service.id),
                )
              }
            />
            {service.label}
          </Label>
        ))}
      </fieldset>
      <Button
        size="sm"
        variant="outline"
        type="submit"
        disabled={saving || !form.formState.isDirty}
      >
        Save guest access
      </Button>
    </form>
  );
}
