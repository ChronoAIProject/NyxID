import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { pendingSetupRequest } from "@/lib/platform-onboarding";
import { useState } from "react";
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
  const { t } = useUiTranslation();
  const search = useRouterState({ select: (s) => s.location.search });
  const hint = onboardingSearch(search).template;
  const templates = useOnboardingTemplates();
  const user = useAuthStore((s) => s.user);
  return (
    <main className="mx-auto max-w-3xl space-y-6 px-4 py-10">
      <PageHeader
        title={t("Your own Telegram assistant")}
        description={t("Choose a starter, review the answers and limits, then connect a bot you own.")}
      />
      {templates.isPending && (
        <p role="status" className="text-12 text-muted-foreground">{t("Loading starters…")}</p>
      )}
      {templates.error && (
        <ErrorBanner
          message={t("Could not load starters.")}
          onRetry={() => void templates.refetch()}
        />
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        {templates.data?.map((template) => (
          <section
            key={template.id}
            className="space-y-4 rounded-xl border border-border bg-card p-4"
          >
            <h2 className="text-15 font-semibold">{template.name}</h2>
            <p className="text-12 text-muted-foreground">
              {template.description}
            </p>
            <p className="text-11 text-muted-foreground">{t("You review all public content. No connector access is included.")}</p>
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
              >{t("Try this starter")}</a>
            </Button>
          </section>
        ))}
      </div>
      <Button variant="link" asChild>
        <Link to="/onboarding/setups">{t("Resume your saved setups")}</Link>
      </Button>
    </main>
  );
}

export function PlatformBindingReviewPage() {
  const search = useRouterState({ select: (s) => s.location.search });
  const code = onboardingSearch(search).code ?? "";
  const user = useAuthStore((s) => s.user);
  return (
    <BindingReview key={`${user?.id ?? "anonymous"}:${code}`} code={code} />
  );
}

function BindingReview({ code }: { code: string }) {
  const { t } = useUiTranslation();
  const user = useAuthStore((s) => s.user);
  const preview = useBindingPreview(code);
  const decision = useBindingDecision(code);
  const [matchedCode, setMatchedCode] = useState("");
  const [replace, setReplace] = useState(false);
  const returnTo = `${window.location.origin}/onboarding/link?code=${encodeURIComponent(code)}`;
  return (
    <main className="mx-auto max-w-lg space-y-6 px-4 py-10">
      <meta name="referrer" content="no-referrer" />
      <PageHeader
        title={t("Review Telegram account linking")}
        description={t("Only continue if you requested this code in the manager's private chat.")}
      />
      {!code && (
        <ErrorBanner message={t("This link has no valid code. Return to the manager and send /link.")} />
      )}
      {preview.isPending && code && (
        <p role="status" className="text-12 text-muted-foreground">{t("Checking your request…")}</p>
      )}
      {preview.error && (
        <ErrorBanner message={t("This link is unavailable or expired. Return to the manager and send /link to start again.")} />
      )}
      {preview.data && (
        <DetailSection title={t("What you are linking")}>
          <div className="space-y-3 p-4 text-12">
            <p>{t("Manager:")}{" "}
              <span className="font-mono">{preview.data.manager_bot_id}</span>
            </p>
            <p>{t("Telegram sender:")}{" "}
              <span className="font-mono">
                {preview.data.requesting_sender_id}
              </span>
            </p>
            <p className="text-muted-foreground">{preview.data.scope}</p>
            <p>{t("Expires: ")}{new Date(preview.data.expires_at).toLocaleString(getFormattingLocale())}</p>
            {preview.data.state === "bound" ? (
              <p role="status">{t("This request is already linked. Manage or revoke it in your saved setups.")}</p>
            ) : preview.data.state === "awaiting_chat_confirmation" ? (
              <p role="status">{t("Browser review complete. Return to the same Telegram private chat and send ")}<code>/confirm {code}</code>.
              </p>
            ) : (
              <>
                {user ? (
                  <>
                    <p>{t("Linking to ")}<strong>{user.email}</strong>{t(". To use another account, sign out before continuing.")}</p>
                    <label className="block space-y-2">{t("Type the code displayed in the initiating Telegram chat")}<Input
                        data-sensitive
                        autoComplete="off"
                        value={matchedCode}
                        onChange={(event) => setMatchedCode(event.target.value)}
                        maxLength={32}
                      />
                    </label>
                    <label className="flex items-start gap-2">
                      <Checkbox
                        checked={replace}
                        onCheckedChange={(value) => setReplace(value === true)}
                      />
                      <span>{t("Replace this chat's existing account binding, if it has one. The previous account's setups remain theirs.")}</span>
                    </label>
                    <div className="flex flex-wrap justify-end gap-2">
                      <Button
                        disabled={!matchedCode || decision.isPending}
                        onClick={() =>
                          decision.mutate({
                            approve: false,
                            matched_code: matchedCode,
                            replace_existing: false,
                          })
                        }
                      >{t("Deny")}</Button>
                      <Button
                        variant="primary"
                        disabled={!matchedCode || decision.isPending}
                        isLoading={decision.isPending}
                        onClick={() =>
                          decision.mutate({
                            approve: true,
                            matched_code: matchedCode,
                            replace_existing: replace,
                          })
                        }
                      >{t("Approve account linking")}</Button>
                    </div>
                  </>
                ) : (
                  <Button variant="primary" asChild>
                    <a
                      href={`/login?return_to=${encodeURIComponent(returnTo)}`}
                    >{t("Sign in or register to review")}</a>
                  </Button>
                )}
              </>
            )}
          </div>
        </DetailSection>
      )}
      {decision.error && <ErrorBanner message={decision.error.message} />}
      <p className="text-12 text-muted-foreground">{t("Linking shares onboarding progress only. It does not deliver credentials, grant tools, create resources or activate a bot.")}</p>
    </main>
  );
}

export function OwnedOnboardingBindings() {
  const actor = useAuthStore((s) => s.user?.id);
  return actor ? <OwnedBindings key={actor} /> : null;
}
function OwnedBindings() {
  const { t } = useUiTranslation();
  const bindings = useOwnedBindings();
  const revoke = useRevokeBinding();
  const [review, setReview] = useState<OwnedBinding>();
  return (
    <DetailSection title={t("Telegram onboarding bindings")}>
      <div className="space-y-3 p-4 text-12">
        {bindings.isPending && <p role="status">{t("Loading bindings…")}</p>}
        {bindings.error && (
          <ErrorBanner
            message={t("Could not load your bindings.")}
            onRetry={() => void bindings.refetch()}
          />
        )}
        {bindings.data?.length === 0 && (
          <p className="text-muted-foreground">{t("No active onboarding bindings. Use /link in the manager's private chat to connect your account.")}</p>
        )}
        {bindings.data?.map((binding) => (
          <div
            key={binding.id}
            className="space-y-2 rounded-lg border border-border p-3"
          >
            <p>{t("Manager ")}{binding.manager_bot_id}{t(" · Telegram sender")}{" "}
              {binding.telegram_sender_id}
            </p>
            <p className="text-muted-foreground">{t("Expires ")}{new Date(binding.expires_at).toLocaleString(getFormattingLocale())}
            </p>
            {review?.id === binding.id ? (
              <>
                <p>{t("Revoke this chat's onboarding access and pause its linked setups?")}</p>
                {review.generation !== binding.generation && (
                  <>
                    <p>{t("This binding changed. Review the new binding before revoking.")}</p>
                    <Button onClick={() => setReview(binding)}>{t("Review current binding")}</Button>
                  </>
                )}
                <div className="flex justify-end gap-2">
                  <Button onClick={() => setReview(undefined)}>{t("Keep binding")}</Button>
                  <Button
                    variant="destructive"
                    isLoading={revoke.isPending}
                    disabled={review.generation !== binding.generation}
                    onClick={() =>
                      revoke.mutate(review, {
                        onSuccess: () => setReview(undefined),
                      })
                    }
                  >{t("Revoke binding")}</Button>
                </div>
              </>
            ) : (
              <Button onClick={() => setReview(binding)}>{t("Review revocation")}</Button>
            )}
          </div>
        ))}
        {revoke.error && <ErrorBanner message={revoke.error.message} />}
      </div>
    </DetailSection>
  );
}

export function CreateSetupForm({
  template,
  actor,
}: {
  template: SetupTemplate;
  actor: string;
}) {
  const { t } = useUiTranslation();
  const create = useSetupMutation();
  const [request] = useState(() => pendingSetupRequest(actor, template));
  const form = useAppForm({
    resolver: zodResolver(setupFactsSchema),
    defaultValues: blankFacts(),
  });
  return (
    <DetailSection
      title={t("Start {{value}} setup", { value: template === "restaurant-starter" ? t("a restaurant") : t("an assistant") })}
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
          <p className="text-12 text-muted-foreground">{t("Start with a name and timezone. Add the remaining details in your saved draft before review. Organization setup is unavailable for this starter.")}</p>
          {(["name", "timezone"] as const).map((name) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    {name === "name"
                      ? t("Assistant or restaurant name")
                      : t("Timezone")}
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
            >{t("Save draft")}</Button>
          </div>
        </form>
      </Form>
    </DetailSection>
  );
}

function ArchivedSetups() {
  const { t } = useUiTranslation();
  const [cursor, setCursor] = useState<{ ms: number; id: string }>();
  const archives = useArchivedSetups(cursor);
  return (
    <DetailSection title={t("Cancelled and expired setups")}>
      <div className="space-y-3 p-4 text-12">
        {archives.error && (
          <ErrorBanner message={t("Could not load archived setups.")} />
        )}
        {archives.isPending && <p role="status">{t("Loading archived setups…")}</p>}
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
              >{t("View saved setup")}</Link>
            </Button>
          </div>
        ))}
        {archives.data?.setups.length === 0 && (
          <p className="text-muted-foreground">{t("No archived setups on this page.")}</p>
        )}
        <div className="flex justify-end gap-2">
          {cursor && (
            <Button onClick={() => setCursor(undefined)}>{t("Newest archived setups")}</Button>
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
              >{t("Older archived setups")}</Button>
            )}
        </div>
      </div>
    </DetailSection>
  );
}

export function AnonymousDraftReview({ binding }: { binding: OwnedBinding }) {
  const { t } = useUiTranslation();
  const query = useAnonymousDrafts(binding);
  const mutation = useAdoptAnonymousDraft(binding);
  const [review, setReview] = useState<PlatformSetup | null>(null);
  return (
    <div className="space-y-3">
      {query.isPending && (
        <p role="status">{t("Loading drafts from this exact chat…")}</p>
      )}
      {query.error && (
        <ErrorBanner message={t("Could not load drafts for this chat. Refresh your binding before trying again.")} />
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {query.data?.map((draft) => (
        <div key={draft.id} className="space-y-2 rounded-lg border p-3">
          <p className="font-medium">
            {draft.facts.name}{t(" · revision ")}{draft.revision}
          </p>
          <dl className="space-y-2">
            {Object.entries(draft.facts).map(([name, value]) => (
              <div key={name}>
                <dt>{name.replaceAll("_", " ")}</dt>
                <dd className="whitespace-pre-wrap break-words">
                  {Array.isArray(value)
                    ? value
                        .map((entry) => `${entry.question}: ${entry.answer}`)
                        .join("\n") || t("None")
                    : value || t("None")}
                </dd>
              </div>
            ))}
          </dl>
          <label className="flex gap-2">
            <Checkbox
              checked={
                review?.id === draft.id && review.revision === draft.revision
              }
              onCheckedChange={(checked) =>
                setReview(checked === true ? draft : null)
              }
            />
            <span>{t("I recognize these facts from my private Telegram chat and want this draft in my account.")}</span>
          </label>
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
          >{t("Adopt reviewed draft")}</Button>
        </div>
      ))}
      {query.data?.length === 0 && (
        <p>{t("No unowned drafts remain for this chat.")}</p>
      )}
    </div>
  );
}

function AnonymousDrafts() {
  const { t } = useUiTranslation();
  const bindings = useOwnedBindings();
  const [selectedId, setSelectedId] = useState("");
  const selected = bindings.data?.find((binding) => binding.id === selectedId);
  return (
    <DetailSection title={t("Bring a Telegram draft into your account")}>
      <div className="space-y-3 p-4 text-12">
        <p>{t("Link your private manager chat first. Then choose that exact chat and review its unowned drafts. Adoption saves a draft; it does not provision or activate a bot.")}</p>
        <label className="block space-y-2">{t("Linked chat")}<select
            value={selectedId}
            onChange={(event) => setSelectedId(event.target.value)}
            className="h-8 w-full rounded-lg border border-input bg-background px-3"
          >
            <option value="">{t("Choose your linked chat")}</option>
            {bindings.data?.map((binding) => (
              <option key={binding.id} value={binding.id}>{t("Telegram sender ")}{binding.telegram_sender_id}{t(" · manager")}{" "}
                {binding.manager_bot_id}
              </option>
            ))}
          </select>
        </label>
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
  const { t } = useUiTranslation();
  const search = useRouterState({ select: (s) => s.location.search });
  const template = onboardingSearch(search).template;
  const actor = useAuthStore((s) => s.user?.id);
  const setups = usePlatformSetups();
  return (
    <div className="space-y-6">
      <PageHeader
        title={t("Your bot setups")}
        description={t("Resume your saved drafts and review changes before connecting or publishing.")}
        actions={
          <Button asChild>
            <Link to="/onboarding">{t("Choose a starter")}</Link>
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
        <p role="status" className="text-12">{t("Loading saved setups…")}</p>
      )}
      {setups.error && (
        <ErrorBanner
          message={t("Could not load your saved setups.")}
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
              <h2 className="text-13 font-semibold">{setup.facts.name}</h2>
              <Badge
                variant={setup.state === "active" ? "success" : "secondary"}
              >
                {setupStatus[setup.state]}
              </Badge>
            </div>
            <p className="text-11 text-muted-foreground">{t("Revision ")}{setup.revision} ·{" "}
              {setup.template === "restaurant-starter"
                ? t("Restaurant starter")
                : t("Telegram assistant")}
            </p>
            <Button asChild>
              <Link
                to="/onboarding/setups/$setupId"
                params={{ setupId: setup.id }}
              >{t("Resume setup")}</Link>
            </Button>
          </section>
        ))}
      </div>
      {setups.data?.length === 0 && !template && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">{t("Your saved setups will appear here. Choose a starter to begin.")}</p>
      )}
      {actor && <AnonymousDrafts key={actor} />}
      {actor && <ArchivedSetups key={actor} />}
      <OwnedOnboardingBindings />
    </div>
  );
}
