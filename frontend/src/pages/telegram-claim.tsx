import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { ArrowLeft } from "lucide-react";
import { useTelegramClaim } from "@/hooks/use-telegram-claim";
import { useAuthStore } from "@/stores/auth-store";
import {
  telegramClaimCodeSchema,
  telegramNewBeginSchema,
} from "@/schemas/telegram-new";
import {
  clearTelegramClaimHandoff,
  readTelegramClaimHandoff,
} from "@/lib/telegram-claim-handoff";
import { ApiError } from "@/lib/api-client";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAppForm } from "@/components/ui/form";

const EXISTING_SETUP =
  "Finish or cancel your existing Telegram creation request first";

export function TelegramClaimPage() {
  const actor = useAuthStore((state) => state.user?.id);
  if (!actor) return <p role="status">Loading your account…</p>;
  return <TelegramClaimForm key={actor} />;
}

export function TelegramClaimForm() {
  const navigate = useNavigate();
  const [code, setCode] = useState(
    () => readTelegramClaimHandoff()?.code ?? "",
  );
  const [deadline, setDeadline] = useState(
    () => readTelegramClaimHandoff()?.expiresAt ?? 0,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  const active = useRef(false);
  const started = useRef(false);
  const form = useAppForm<{ label: string; target_org_id?: string }>({
    resolver: zodResolver(telegramNewBeginSchema),
    defaultValues: { label: "" },
  });
  const label = useWatch({ control: form.control, name: "label" });
  const orgId = useWatch({ control: form.control, name: "target_org_id" });
  const { preview, redeem, existing, cancel } = useTelegramClaim(
    code,
    label,
    orgId,
  );
  const busy = preview.isPending || redeem.isPending || cancel.isPending;
  const pending = existing.data?.request;
  const conflict = redeem.error?.message === EXISTING_SETUP;

  function discardCode() {
    clearTelegramClaimHandoff();
    setCode("");
    setDeadline(0);
  }

  async function checkCode() {
    const result = telegramClaimCodeSchema.safeParse(code);
    if (!result.success) {
      setNotice(result.error.issues[0]?.message ?? "Enter your claim code");
      return;
    }
    setNotice(null);
    try {
      const value = await preview.mutateAsync();
      if (!active.current) return;
      setDeadline(Date.parse(value.expires_at));
      if (!form.getValues("label"))
        form.setValue("label", value.bot_username, {
          shouldDirty: false,
          shouldTouch: false,
        });
    } catch (error) {
      if (active.current && error instanceof ApiError && error.status === 404) {
        discardCode();
        setNotice(error.message);
        preview.reset();
      }
    }
  }

  useEffect(() => {
    active.current = true;
    clearTelegramClaimHandoff();
    // Start after StrictMode has restored the mutation observer subscription.
    queueMicrotask(() => {
      if (active.current && !started.current) {
        started.current = true;
        if (code) void checkCode();
      }
    });
    return () => {
      active.current = false;
      clearTelegramClaimHandoff();
    };
    // Capture and preview the initial handoff once, including under StrictMode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!deadline) return;
    const timer = window.setTimeout(
      () => {
        discardCode();
        preview.reset();
        redeem.reset();
        setNotice(
          "This claim code expired. Send /recover @YourBotUsername in the manager chat for a new code.",
        );
      },
      Math.max(0, deadline - Date.now()),
    );
    return () => window.clearTimeout(timer);
    // Mutation objects change on every render; only the expiry schedules this timer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deadline]);

  async function connect() {
    if (!preview.data || !code || busy) return;
    setLocked(true);
    try {
      const request = await redeem.mutateAsync();
      if (!active.current) return;
      discardCode();
      preview.reset();
      redeem.reset();
      await navigate({
        to: "/channel-bots",
        search: { connect: "telegram-new", request_id: request.id },
        replace: true,
      });
    } catch (error) {
      if (!active.current) return;
      if (error instanceof ApiError && error.status === 404) {
        discardCode();
        setNotice(error.message);
        preview.reset();
        redeem.reset();
        setLocked(false);
      }
      if (error instanceof Error && error.message === EXISTING_SETUP) {
        setLocked(false);
        void existing.refetch();
      }
    }
  }

  return (
    <section
      aria-label="Connect Telegram bot"
      className="mx-auto w-full max-w-2xl space-y-6 pb-6"
    >
      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          discardCode();
          void navigate({ to: "/channel-bots", search: {}, replace: true });
        }}
      >
        <ArrowLeft className="size-4" /> Back to Channel Bots
      </Button>
      <PageHeader
        title={
          preview.data
            ? `Connect @${preview.data.bot_username}`
            : "Connect your Telegram bot"
        }
        description="Choose where your bot will appear in NyxID, then connect it."
      />
      {(notice || preview.error || redeem.error || cancel.error) && (
        <ErrorBanner
          message={
            notice ??
            cancel.error?.message ??
            redeem.error?.message ??
            preview.error?.message ??
            "Unable to connect"
          }
        />
      )}
      {!preview.data ? (
        <form
          key="claim-code"
          className="space-y-4 rounded-xl border border-border bg-card p-4 sm:p-5"
          onSubmit={(event) => {
            event.preventDefault();
            void checkCode();
          }}
        >
          <div className="space-y-2">
            <Label htmlFor="telegram-claim-code">Claim code</Label>
            <Input
              id="telegram-claim-code"
              data-sensitive
              type="password"
              autoComplete="off"
              maxLength={64}
              value={code}
              disabled={busy}
              onChange={(event) => {
                setCode(event.target.value);
                preview.reset();
                redeem.reset();
                setLocked(false);
                setNotice(null);
              }}
            />
            <p className="text-xs text-muted-foreground">
              Use the code from your private Telegram setup chat. Only enter
              codes for bots you created.
            </p>
          </div>
          <Button
            type="submit"
            variant="primary"
            isLoading={preview.isPending}
            disabled={busy || !code.trim()}
          >
            {preview.error ? "Retry" : "Check code"}
          </Button>
        </form>
      ) : (
        <form
          key="claim-destination"
          className="space-y-4 rounded-xl border border-border bg-card p-4 sm:p-5"
          onSubmit={form.handleSubmit(connect)}
        >
          <div className="space-y-2">
            <Label htmlFor="telegram-claim-label">Bot label in NyxID</Label>
            <Input
              id="telegram-claim-label"
              maxLength={128}
              disabled={locked || busy}
              {...form.register("label")}
            />
            {form.formState.errors.label && (
              <p className="text-xs text-destructive">
                {form.formState.errors.label.message}
              </p>
            )}
          </div>
          <div className="space-y-2">
            <Label>Connect to</Label>
            <OrgScopeSelect
              label="Connect to"
              value={orgId ?? null}
              disabled={locked || busy}
              onChange={(next) =>
                form.setValue("target_org_id", next ?? undefined)
              }
            />
            <p className="text-xs text-muted-foreground">
              Your personal account or an organization you manage.
            </p>
          </div>
          <p className="text-xs text-muted-foreground">
            Once connected, choose an AI agent in the bot’s settings to handle
            replies.
          </p>
          <Button
            type="submit"
            variant="primary"
            isLoading={redeem.isPending}
            disabled={busy || !code || !label.trim()}
          >
            {locked && redeem.error ? "Retry connection" : "Connect"}
          </Button>
        </form>
      )}
      {conflict && pending && (
        <div className="space-y-3 rounded-xl border border-border bg-card p-4">
          <p className="text-sm">Existing setup: {pending.label}</p>
          <Button
            variant="outline"
            onClick={() =>
              void navigate({
                to: "/channel-bots",
                search: { connect: "telegram-new", request_id: pending.id },
              })
            }
          >
            Resume existing setup
          </Button>
          {!["provisioning", "connected", "suspended"].includes(
            pending.status,
          ) && (
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => {
                void cancel
                  .mutateAsync(pending.id)
                  .then(() => {
                    if (active.current) redeem.reset();
                  })
                  .catch(() => {});
              }}
            >
              Cancel setup
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
