import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { Label } from "@/components/ui/label";
import { pendingSetupRequest } from "@/lib/platform-onboarding";
import { useEffect, useLayoutEffect, useState } from "react";
import { CopyableField } from "@/components/shared/copyable-field";
import {
  captureOnboardingLink,
  readOnboardingLink,
  subscribeOnboardingLink,
  clearOnboardingLink,
  onboardingConfirmationUrl,
} from "@/lib/platform-onboarding-handoff";
import { useRouterState, Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAuthStore } from "@/stores/auth-store";
import {
  useArchivedSetups,
  useAnonymousDrafts,
  useAdoptAnonymousDraft,
  useBindingDecision,
  useBindingPreview,
  useOnboardingTemplates,
  useOwnedBindings,
  usePlatformSetups,
  useRevokeBinding,
  useSetupMutation,
} from "@/hooks/use-platform-onboarding";
import {
  blankFacts,
  onboardingSearch,
  setupFactsSchema,
  setupStatus,
  type SetupTemplate,
  type OwnedBinding,
  type PlatformSetup,
} from "@/schemas/platform-onboarding";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { DetailSection } from "@/components/shared/detail-section";

export function PlatformOnboardingPage() {
  useUiTranslation();
  const search = useRouterState({ select: (s) => s.location.search });
  const hint = onboardingSearch(search).template;
  const templates = useOnboardingTemplates();
  const user = useAuthStore((s) => s.user);
  return (
    <main className="mx-auto max-w-3xl space-y-6 px-4 py-10">
      <PageHeader
        title={translateUi("Your own Telegram assistant")}
        description={translateUi("Choose a starter, review the answers and limits, then connect a bot you own.")}
      />
      {templates.isPending && (
        <p role="status" className="text-14 text-muted-foreground">{translateUi("Loading starters…")}</p>
      )}
      {templates.error && (
        <ErrorBanner
          message={translateUi("Could not load starters.")}
          onRetry={() => void templates.refetch()}
        />
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        {templates.data?.map((template) => (
          <section
            key={template.id}
            className="space-y-4 rounded-xl border border-border bg-card p-4"
          >
            <h2 className="type-card-title">{template.name}</h2>
            <p className="text-14 text-muted-foreground">
              {template.description}
            </p>
            <p className="text-13 text-muted-foreground">{translateUi("You review all public content. No connector access is included.")}</p>
            <Button
              variant={hint === template.id ? "primary" : "outline"}
              asChild
            >
              <a
                href={
                  user
                    ? `/onboarding/setups?template=${template.id}`
                    : `/login?return_to=${encodeURIComponent(`${window.location.origin}/onboarding/setups?template=${template.id}`)}`
                }
              >{translateUi("Try this starter")}</a>
            </Button>
          </section>
        ))}
      </div>
      <Button variant="link" asChild>
        <Link to="/onboarding/setups">{translateUi("Resume your saved setups")}</Link>
      </Button>
    </main>
  );
}

export function PlatformBindingReviewPage() {
  useUiTranslation();
  const location = useRouterState({ select: (s) => s.location.href });
  const [handoff, setHandoff] = useState(readOnboardingLink);
  useLayoutEffect(() => {
    const unsubscribe = subscribeOnboardingLink(setHandoff);
    captureOnboardingLink();
    return unsubscribe;
  }, [location]);
  const user = useAuthStore((s) => s.user);
  return (
    <BindingReview
      key={`${user?.id ?? "anonymous"}:${handoff?.id ?? "missing"}`}
      code={handoff?.code ?? ""}
    />
  );
}

function BindingReview({ code }: { code: string }) {
  useUiTranslation();
  const user = useAuthStore((s) => s.user);
  const preview = useBindingPreview(code);
  const decision = useBindingDecision(code);
  const [replace, setReplace] = useState(false);
  const [now, setNow] = useState(Date.now);
  const state = preview.data?.state;
  const expired = !!preview.data && Date.parse(preview.data.expires_at) <= now;
  const command = decision.data?.confirmation_command;
  const telegramReturn = onboardingConfirmationUrl(
    command,
    preview.data?.manager_username,
  );
  const returnTo = `${window.location.origin}/onboarding/link`;
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    if (state === "bound" || state === "denied" || expired)
      clearOnboardingLink();
  }, [state, expired]);
  return (
    <main className="mx-auto max-w-lg space-y-6 px-4 py-10" data-sensitive>
      <meta name="referrer" content="no-referrer" />
      <PageHeader
        title={translateUi("Review Telegram account linking")}
        description={translateUi("Continue only if you requested this in the manager's private chat.")}
      />
      {(!code || expired || preview.error) && (
        <ErrorBanner message={translateUi("This link is unavailable or expired. Return to the manager's private chat and use /help to start a new onboarding request.")} />
      )}
      {preview.isPending && code && (
        <p role="status" className="text-14 text-muted-foreground">{translateUi("Checking your request…")}</p>
      )}
      {preview.data && !expired && (
        <DetailSection title={translateUi("What you are linking")}>
          <div className="space-y-3 p-4 text-14">
            <p>{translateUi("Manager:")}{" "}
              <span className="font-mono">
                {preview.data.manager_username
                  ? `@${preview.data.manager_username}`
                  : preview.data.manager_bot_id}
              </span>
            </p>
            <p>{translateUi("Telegram sender:")}{" "}
              <span className="font-mono">
                {preview.data.requesting_sender_id}
              </span>
            </p>
            <p className="text-muted-foreground">{preview.data.scope}</p>
            <p>{translateUi("Expires: ")}{new Date(preview.data.expires_at).toLocaleString(getFormattingLocale())}</p>
            {state === "bound" ? (
              <>
                <p role="status">{translateUi("Telegram account linked.")}</p>
                <Button variant="primary" asChild>
                  <Link to="/onboarding/setups">{translateUi("Open your saved setups")}</Link>
                </Button>
              </>
            ) : state === "denied" || decision.data?.state === "denied" ? (
              <p role="status">{translateUi("Linking was cancelled. No access was granted.")}</p>
            ) : state === "awaiting_chat_confirmation" || command ? (
              <div className="space-y-3">
                <p role="status">{translateUi("Browser review complete. Confirm from the same Telegram private chat to finish linking.")}</p>
                {command ? (
                  <>
                    {telegramReturn && (
                      <Button variant="primary" asChild>
                        <a
                          href={telegramReturn}
                          target="_blank"
                          rel="noopener noreferrer"
                        >{translateUi("Open Telegram to finish")}</a>
                      </Button>
                    )}
                    <CopyableField
                      label={translateUi("Confirmation command")}
                      value={command}
                    />
                  </>
                ) : (
                  <p>{translateUi("The confirmation is shown only after approval. If this page was reloaded or the account changed, start a new onboarding request in Telegram.")}</p>
                )}
                <Button
                  onClick={() => void preview.refetch()}
                  disabled={preview.isFetching}
                >{translateUi("Refresh status")}</Button>
              </div>
            ) : user ? (
              <>
                <p>{translateUi("Linking to ")}<strong>{user.email}</strong>{translateUi(". To use another account, sign out before continuing. ")}</p>
                <Label className="flex items-start gap-2">
                  <Checkbox
                    checked={replace}
                    onCheckedChange={(value) => setReplace(value === true)}
                  />
                  <span>{translateUi("Replace this chat's existing account binding, if it has one. The previous account's setups remain theirs.")}</span>
                </Label>
                <div className="flex flex-wrap justify-end gap-2">
                  <Button
                    disabled={decision.isPending}
                    onClick={() =>
                      decision.mutate({
                        approve: false,
                        replace_existing: false,
                      })
                    }
                  >{translateUi("Deny")}</Button>
                  <Button
                    variant="primary"
                    isLoading={decision.isPending}
                    onClick={() =>
                      decision.mutate({
                        approve: true,
                        replace_existing: replace,
                      })
                    }
                  >{translateUi("Approve account linking")}</Button>
                </div>
              </>
            ) : (
              <Button variant="primary" asChild>
                <a href={`/login?return_to=${encodeURIComponent(returnTo)}`}>{translateUi("Sign in or register to review")}</a>
              </Button>
            )}
          </div>
        </DetailSection>
      )}
      {decision.error && <ErrorBanner message={decision.error.message} />}
      <p className="text-14 text-muted-foreground">{translateUi("Linking shares onboarding progress only. It does not deliver credentials, grant tools, create resources or activate a bot.")}</p>
    </main>
  );
}

export function OwnedOnboardingBindings() {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  return actor ? <OwnedBindings key={actor} /> : null;
}
function OwnedBindings() {
  useUiTranslation();
  const bindings = useOwnedBindings();
  const revoke = useRevokeBinding();
  const [review, setReview] = useState<OwnedBinding>();
  return (
    <GroupedList title={translateUi("Telegram onboarding bindings")} footer={<>
      {bindings.error && <div role="alert" className="space-y-2">
        <p className="text-destructive">{translateUi("Could not load your bindings.")}</p>
        <Button variant="ghost" size="sm" isLoading={bindings.isFetching} onClick={() => void bindings.refetch()}>{translateUi("Retry")}</Button>
      </div>}
      {revoke.error && <p role="alert" className="text-destructive">{revoke.error.message}</p>}
    </>}>
        {bindings.isPending && <GroupedRow title={translateUi("Loading bindings…")} role="status" />}
        {bindings.data?.length === 0 && (
          <GroupedRow title={translateUi("No active onboarding bindings. Use /help in the manager’s private chat to start onboarding.")} />
        )}
        {bindings.data?.map((binding) => (
          <GroupedRow key={binding.id} className="block space-y-2 break-all">
            <p>{translateUi("Manager ")}{binding.manager_bot_id}{translateUi(" · Telegram sender")}{" "}
              {binding.telegram_sender_id}
            </p>
            <p className="text-muted-foreground">{translateUi("Expires ")}{new Date(binding.expires_at).toLocaleString(getFormattingLocale())}
            </p>
            {review?.id === binding.id ? (
              <>
                <p>{translateUi("Revoke this chat's onboarding access and pause its linked setups?")}</p>
                {review.generation !== binding.generation && (
                  <>
                    <p>{translateUi("This binding changed. Review the new binding before revoking.")}</p>
                    <Button onClick={() => setReview(binding)}>{translateUi("Review current binding")}</Button>
                  </>
                )}
                <div className="flex justify-end gap-2">
                  <Button onClick={() => setReview(undefined)}>{translateUi("Keep binding")}</Button>
                  <Button
                    variant="destructive"
                    isLoading={revoke.isPending}
                    disabled={review.generation !== binding.generation}
                    onClick={() =>
                      revoke.mutate(review, {
                        onSuccess: () => setReview(undefined),
                      })
                    }
                  >{translateUi("Revoke binding")}</Button>
                </div>
              </>
            ) : (
              <Button onClick={() => setReview(binding)}>{translateUi("Review revocation")}</Button>
            )}
          </GroupedRow>
        ))}
    </GroupedList>
  );
}

export function CreateSetupForm({
  template,
  actor,
}: {
  template: SetupTemplate;
  actor: string;
}) {
  useUiTranslation();
  const create = useSetupMutation();
  const [request] = useState(() => pendingSetupRequest(actor, template));
  const form = useAppForm({
    resolver: zodResolver(setupFactsSchema),
    defaultValues: blankFacts(),
  });
  return (
    <DetailSection
      title={template === "restaurant-starter" ? translateUi("Start restaurant setup") : translateUi("Start assistant setup")}
    >
      <Form {...form}>
        <form
          className="space-y-4 p-4"
          onSubmit={form.handleSubmit((facts) =>
            create.mutate(
              { action: "create", request_id: request.id, template, facts },
              {
                onSuccess: (saved) => {
                  request.clear();
                  window.location.assign(`/onboarding/setups/${saved.id}`);
                },
              },
            ),
          )}
        >
          <p className="text-14 text-muted-foreground">{translateUi("Start with a name and timezone. Add the remaining details in your saved draft before review. Organization setup is unavailable for this starter.")}</p>
          {(["name", "timezone"] as const).map((name) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    {name === "name"
                      ? translateUi("Assistant or restaurant name")
                      : translateUi("Timezone")}
                  </FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          ))}
          {create.error && <ErrorBanner message={create.error.message} />}
          <div className="flex justify-end">
            <Button
              type="submit"
              variant="primary"
              disabled={!form.formState.isDirty || !form.watch("name")}
              isLoading={create.isPending}
            >{translateUi("Save draft")}</Button>
          </div>
        </form>
      </Form>
    </DetailSection>
  );
}

function ArchivedSetups() {
  useUiTranslation();
  const [cursor, setCursor] = useState<{ ms: number; id: string }>();
  const archives = useArchivedSetups(cursor);
  return (
    <DetailSection title={translateUi("Cancelled and expired setups")}>
      <div className="space-y-3 p-4 text-14">
        {archives.error && (
          <ErrorBanner message={translateUi("Could not load archived setups.")} />
        )}
        {archives.isPending && <p role="status">{translateUi("Loading archived setups…")}</p>}
        {archives.data?.setups.map((setup) => (
          <div
            key={setup.id}
            className="flex flex-wrap items-center justify-between gap-2"
          >
            <span>
              {setup.facts.name} · {setupStatus[setup.state]}
            </span>
            <Button asChild>
              <Link
                to="/onboarding/setups/$setupId"
                params={{ setupId: setup.id }}
              >{translateUi("View saved setup")}</Link>
            </Button>
          </div>
        ))}
        {archives.data?.setups.length === 0 && (
          <p className="text-muted-foreground">{translateUi("No archived setups on this page.")}</p>
        )}
        <div className="flex justify-end gap-2">
          {cursor && (
            <Button onClick={() => setCursor(undefined)}>{translateUi("Newest archived setups")}</Button>
          )}
          {archives.data?.next_before_id &&
            archives.data.next_before_ms !== null && (
              <Button
                onClick={() => {
                  if (
                    archives.data?.next_before_id &&
                    archives.data.next_before_ms !== null
                  )
                    setCursor({
                      ms: archives.data.next_before_ms,
                      id: archives.data.next_before_id,
                    });
                }}
              >{translateUi("Older archived setups")}</Button>
            )}
        </div>
      </div>
    </DetailSection>
  );
}

export function AnonymousDraftReview({ binding }: { binding: OwnedBinding }) {
  useUiTranslation();
  const query = useAnonymousDrafts(binding);
  const mutation = useAdoptAnonymousDraft(binding);
  const [review, setReview] = useState<PlatformSetup | null>(null);
  return (
    <div className="space-y-3">
      {query.isPending && (
        <p role="status">{translateUi("Loading drafts from this exact chat…")}</p>
      )}
      {query.error && (
        <ErrorBanner message={translateUi("Could not load drafts for this chat. Refresh your binding before trying again.")} />
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {query.data?.map((draft) => (
        <div key={draft.id} className="space-y-2 rounded-lg border p-3">
          <p className="font-medium">
            {draft.facts.name}{translateUi(" · revision ")}{draft.revision}
          </p>
          <dl className="space-y-2">
            {Object.entries(draft.facts).map(([name, value]) => (
              <div key={name}>
                <dt>{name.replaceAll("_", " ")}</dt>
                <dd className="whitespace-pre-wrap break-words">
                  {Array.isArray(value)
                    ? value
                        .map((entry) => `${entry.question}: ${entry.answer}`)
                        .join("\n") || translateUi("None")
                    : value || translateUi("None")}
                </dd>
              </div>
            ))}
          </dl>
          <Label className="flex gap-2">
            <Checkbox
              checked={
                review?.id === draft.id && review.revision === draft.revision
              }
              onCheckedChange={(checked) =>
                setReview(checked === true ? draft : null)
              }
            />
            <span>{translateUi("I recognize these facts from my private Telegram chat and want this draft in my account.")}</span>
          </Label>
          <Button
            disabled={
              review?.id !== draft.id ||
              review.revision !== draft.revision ||
              mutation.isPending
            }
            onClick={() => {
              if (review)
                mutation.mutate(review, { onSuccess: () => setReview(null) });
            }}
          >{translateUi("Adopt reviewed draft")}</Button>
        </div>
      ))}
      {query.data?.length === 0 && (
        <p>{translateUi("No unowned drafts remain for this chat.")}</p>
      )}
    </div>
  );
}

function AnonymousDrafts() {
  useUiTranslation();
  const bindings = useOwnedBindings();
  const [selectedId, setSelectedId] = useState("");
  const selected = bindings.data?.find((binding) => binding.id === selectedId);
  return (
    <DetailSection title={translateUi("Bring a Telegram draft into your account")}>
      <div className="space-y-3 p-4 text-14">
        <p>{translateUi("Link your private manager chat first. Then choose that exact chat and review its unowned drafts. Adoption saves a draft; it does not provision or activate a bot.")}</p>
        <Label className="block space-y-2">{translateUi("Linked chat")}<select
            value={selectedId}
            onChange={(event) => setSelectedId(event.target.value)}
            className="h-8 w-full rounded-lg border border-input bg-background px-3"
          >
            <option value="">{translateUi("Choose your linked chat")}</option>
            {bindings.data?.map((binding) => (
              <option key={binding.id} value={binding.id}>{translateUi("Telegram sender ")}{binding.telegram_sender_id}{translateUi(" · manager")}{" "}
                {binding.manager_bot_id}
              </option>
            ))}
          </select>
        </Label>
        {selected && (
          <AnonymousDraftReview
            key={`${selected.id}:${selected.generation}`}
            binding={selected}
          />
        )}
      </div>
    </DetailSection>
  );
}

export function PlatformSetupsPage() {
  useUiTranslation();
  const search = useRouterState({ select: (s) => s.location.search });
  const template = onboardingSearch(search).template;
  const actor = useAuthStore((s) => s.user?.id);
  const setups = usePlatformSetups();
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Your bot setups")}
        description={translateUi("Resume your saved drafts and review changes before connecting or publishing.")}
        actions={
          <Button asChild>
            <Link to="/onboarding">{translateUi("Choose a starter")}</Link>
          </Button>
        }
      />
      {template && actor && (
        <CreateSetupForm
          key={`${actor}:${template}`}
          template={template}
          actor={actor}
        />
      )}
      {setups.isPending && (
        <p role="status" className="text-14">{translateUi("Loading saved setups…")}</p>
      )}
      {setups.error && (
        <ErrorBanner
          message={translateUi("Could not load your saved setups.")}
          onRetry={() => void setups.refetch()}
        />
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        {setups.data?.map((setup) => (
          <section
            key={setup.id}
            className="space-y-3 rounded-xl border border-border bg-card p-4"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="type-card-title">{setup.facts.name}</h2>
              <Badge
                variant={setup.state === "active" ? "success" : "secondary"}
              >
                {setupStatus[setup.state]}
              </Badge>
            </div>
            <p className="text-13 text-muted-foreground">{translateUi("Revision ")}{setup.revision} ·{" "}
              {setup.template === "restaurant-starter"
                ? translateUi("Restaurant starter")
                : translateUi("Telegram assistant")}
            </p>
            <Button asChild>
              <Link
                to="/onboarding/setups/$setupId"
                params={{ setupId: setup.id }}
              >{translateUi("Resume setup")}</Link>
            </Button>
          </section>
        ))}
      </div>
      {setups.data?.length === 0 && !template && (
        <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">{translateUi("Your saved setups will appear here. Choose a starter to begin.")}</p>
      )}
      {actor && <AnonymousDrafts key={actor} />}
      {actor && <ArchivedSetups key={actor} />}
      <OwnedOnboardingBindings />
    </div>
  );
}
