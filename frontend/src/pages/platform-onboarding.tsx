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
  const search = useRouterState({ select: (s) => s.location.search });
  const hint = onboardingSearch(search).template;
  const templates = useOnboardingTemplates();
  const user = useAuthStore((s) => s.user);
  return (
    <main className="mx-auto max-w-3xl space-y-6 px-4 py-10">
      <PageHeader
        title="Your own Telegram assistant"
        description="Choose a starter, review the answers and limits, then connect a bot you own."
      />
      {templates.isPending && (
        <p role="status" className="text-12 text-muted-foreground">
          Loading starters…
        </p>
      )}
      {templates.error && (
        <ErrorBanner
          message="Could not load starters."
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
            <p className="text-11 text-muted-foreground">
              You review all public content. No connector access is included.
            </p>
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
              >
                Try this starter
              </a>
            </Button>
          </section>
        ))}
      </div>
      <Button variant="link" asChild>
        <Link to="/onboarding/setups">Resume your saved setups</Link>
      </Button>
    </main>
  );
}

export function PlatformBindingReviewPage() {
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
        title="Review Telegram account linking"
        description="Continue only if you requested this in the manager's private chat."
      />
      {(!code || expired || preview.error) && (
        <ErrorBanner message="This link is unavailable or expired. Return to the manager's private chat and use /help to start a new onboarding request." />
      )}
      {preview.isPending && code && (
        <p role="status" className="text-12 text-muted-foreground">
          Checking your request…
        </p>
      )}
      {preview.data && !expired && (
        <DetailSection title="What you are linking">
          <div className="space-y-3 p-4 text-12">
            <p>
              Manager:{" "}
              <span className="font-mono">
                {preview.data.manager_username
                  ? `@${preview.data.manager_username}`
                  : preview.data.manager_bot_id}
              </span>
            </p>
            <p>
              Telegram sender:{" "}
              <span className="font-mono">
                {preview.data.requesting_sender_id}
              </span>
            </p>
            <p className="text-muted-foreground">{preview.data.scope}</p>
            <p>Expires: {new Date(preview.data.expires_at).toLocaleString()}</p>
            {state === "bound" ? (
              <>
                <p role="status">Telegram account linked.</p>
                <Button variant="primary" asChild>
                  <Link to="/onboarding/setups">Open your saved setups</Link>
                </Button>
              </>
            ) : state === "denied" || decision.data?.state === "denied" ? (
              <p role="status">Linking was cancelled. No access was granted.</p>
            ) : state === "awaiting_chat_confirmation" || command ? (
              <div className="space-y-3">
                <p role="status">
                  Browser review complete. Confirm from the same Telegram
                  private chat to finish linking.
                </p>
                {command ? (
                  <>
                    {telegramReturn && (
                      <Button variant="primary" asChild>
                        <a
                          href={telegramReturn}
                          target="_blank"
                          rel="noopener noreferrer"
                        >
                          Open Telegram to finish
                        </a>
                      </Button>
                    )}
                    <CopyableField
                      label="Confirmation command"
                      value={command}
                    />
                  </>
                ) : (
                  <p>
                    The confirmation is shown only after approval. If this page
                    was reloaded or the account changed, start a new onboarding
                    request in Telegram.
                  </p>
                )}
                <Button
                  onClick={() => void preview.refetch()}
                  disabled={preview.isFetching}
                >
                  Refresh status
                </Button>
              </div>
            ) : user ? (
              <>
                <p>
                  Linking to <strong>{user.email}</strong>. To use another
                  account, sign out before continuing.
                </p>
                <label className="flex items-start gap-2">
                  <Checkbox
                    checked={replace}
                    onCheckedChange={(value) => setReplace(value === true)}
                  />
                  <span>
                    Replace this chat's existing account binding, if it has one.
                    The previous account's setups remain theirs.
                  </span>
                </label>
                <div className="flex flex-wrap justify-end gap-2">
                  <Button
                    disabled={decision.isPending}
                    onClick={() =>
                      decision.mutate({
                        approve: false,
                        replace_existing: false,
                      })
                    }
                  >
                    Deny
                  </Button>
                  <Button
                    variant="primary"
                    isLoading={decision.isPending}
                    onClick={() =>
                      decision.mutate({
                        approve: true,
                        replace_existing: replace,
                      })
                    }
                  >
                    Approve account linking
                  </Button>
                </div>
              </>
            ) : (
              <Button variant="primary" asChild>
                <a href={`/login?return_to=${encodeURIComponent(returnTo)}`}>
                  Sign in or register to review
                </a>
              </Button>
            )}
          </div>
        </DetailSection>
      )}
      {decision.error && <ErrorBanner message={decision.error.message} />}
      <p className="text-12 text-muted-foreground">
        Linking shares onboarding progress only. It does not deliver
        credentials, grant tools, create resources or activate a bot.
      </p>
    </main>
  );
}

export function OwnedOnboardingBindings() {
  const actor = useAuthStore((s) => s.user?.id);
  return actor ? <OwnedBindings key={actor} /> : null;
}
function OwnedBindings() {
  const bindings = useOwnedBindings();
  const revoke = useRevokeBinding();
  const [review, setReview] = useState<OwnedBinding>();
  return (
    <DetailSection title="Telegram onboarding bindings">
      <div className="space-y-3 p-4 text-12">
        {bindings.isPending && <p role="status">Loading bindings…</p>}
        {bindings.error && (
          <ErrorBanner
            message="Could not load your bindings."
            onRetry={() => void bindings.refetch()}
          />
        )}
        {bindings.data?.length === 0 && (
          <p className="text-muted-foreground">
            No active onboarding bindings. Use /help in the manager's private
            chat to start onboarding.
          </p>
        )}
        {bindings.data?.map((binding) => (
          <div
            key={binding.id}
            className="space-y-2 rounded-lg border border-border p-3"
          >
            <p>
              Manager {binding.manager_bot_id} · Telegram sender{" "}
              {binding.telegram_sender_id}
            </p>
            <p className="text-muted-foreground">
              Expires {new Date(binding.expires_at).toLocaleString()}
            </p>
            {review?.id === binding.id ? (
              <>
                <p>
                  Revoke this chat's onboarding access and pause its linked
                  setups?
                </p>
                {review.generation !== binding.generation && (
                  <>
                    <p>
                      This binding changed. Review the new binding before
                      revoking.
                    </p>
                    <Button onClick={() => setReview(binding)}>
                      Review current binding
                    </Button>
                  </>
                )}
                <div className="flex justify-end gap-2">
                  <Button onClick={() => setReview(undefined)}>
                    Keep binding
                  </Button>
                  <Button
                    variant="destructive"
                    isLoading={revoke.isPending}
                    disabled={review.generation !== binding.generation}
                    onClick={() =>
                      revoke.mutate(review, {
                        onSuccess: () => setReview(undefined),
                      })
                    }
                  >
                    Revoke binding
                  </Button>
                </div>
              </>
            ) : (
              <Button onClick={() => setReview(binding)}>
                Review revocation
              </Button>
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
  const create = useSetupMutation();
  const [request] = useState(() => pendingSetupRequest(actor, template));
  const form = useAppForm({
    resolver: zodResolver(setupFactsSchema),
    defaultValues: blankFacts(),
  });
  return (
    <DetailSection
      title={`Start ${template === "restaurant-starter" ? "a restaurant" : "an assistant"} setup`}
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
          <p className="text-12 text-muted-foreground">
            Start with a name and timezone. Add the remaining details in your
            saved draft before review. Organization setup is unavailable for
            this starter.
          </p>
          {(["name", "timezone"] as const).map((name) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    {name === "name"
                      ? "Assistant or restaurant name"
                      : "Timezone"}
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
            >
              Save draft
            </Button>
          </div>
        </form>
      </Form>
    </DetailSection>
  );
}

function ArchivedSetups() {
  const [cursor, setCursor] = useState<{ ms: number; id: string }>();
  const archives = useArchivedSetups(cursor);
  return (
    <DetailSection title="Cancelled and expired setups">
      <div className="space-y-3 p-4 text-12">
        {archives.error && (
          <ErrorBanner message="Could not load archived setups." />
        )}
        {archives.isPending && <p role="status">Loading archived setups…</p>}
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
              >
                View saved setup
              </Link>
            </Button>
          </div>
        ))}
        {archives.data?.setups.length === 0 && (
          <p className="text-muted-foreground">
            No archived setups on this page.
          </p>
        )}
        <div className="flex justify-end gap-2">
          {cursor && (
            <Button onClick={() => setCursor(undefined)}>
              Newest archived setups
            </Button>
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
              >
                Older archived setups
              </Button>
            )}
        </div>
      </div>
    </DetailSection>
  );
}

export function AnonymousDraftReview({ binding }: { binding: OwnedBinding }) {
  const query = useAnonymousDrafts(binding);
  const mutation = useAdoptAnonymousDraft(binding);
  const [review, setReview] = useState<PlatformSetup | null>(null);
  return (
    <div className="space-y-3">
      {query.isPending && (
        <p role="status">Loading drafts from this exact chat…</p>
      )}
      {query.error && (
        <ErrorBanner message="Could not load drafts for this chat. Refresh your binding before trying again." />
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {query.data?.map((draft) => (
        <div key={draft.id} className="space-y-2 rounded-lg border p-3">
          <p className="font-medium">
            {draft.facts.name} · revision {draft.revision}
          </p>
          <dl className="space-y-2">
            {Object.entries(draft.facts).map(([name, value]) => (
              <div key={name}>
                <dt>{name.replaceAll("_", " ")}</dt>
                <dd className="whitespace-pre-wrap break-words">
                  {Array.isArray(value)
                    ? value
                        .map((entry) => `${entry.question}: ${entry.answer}`)
                        .join("\n") || "None"
                    : value || "None"}
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
            <span>
              I recognize these facts from my private Telegram chat and want
              this draft in my account.
            </span>
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
          >
            Adopt reviewed draft
          </Button>
        </div>
      ))}
      {query.data?.length === 0 && (
        <p>No unowned drafts remain for this chat.</p>
      )}
    </div>
  );
}

function AnonymousDrafts() {
  const bindings = useOwnedBindings();
  const [selectedId, setSelectedId] = useState("");
  const selected = bindings.data?.find((binding) => binding.id === selectedId);
  return (
    <DetailSection title="Bring a Telegram draft into your account">
      <div className="space-y-3 p-4 text-12">
        <p>
          Link your private manager chat first. Then choose that exact chat and
          review its unowned drafts. Adoption saves a draft; it does not
          provision or activate a bot.
        </p>
        <label className="block space-y-2">
          Linked chat
          <select
            value={selectedId}
            onChange={(event) => setSelectedId(event.target.value)}
            className="h-8 w-full rounded-lg border border-input bg-background px-3"
          >
            <option value="">Choose your linked chat</option>
            {bindings.data?.map((binding) => (
              <option key={binding.id} value={binding.id}>
                Telegram sender {binding.telegram_sender_id} · manager{" "}
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
  const search = useRouterState({ select: (s) => s.location.search });
  const template = onboardingSearch(search).template;
  const actor = useAuthStore((s) => s.user?.id);
  const setups = usePlatformSetups();
  return (
    <div className="space-y-6">
      <PageHeader
        title="Your bot setups"
        description="Resume your saved drafts and review changes before connecting or publishing."
        actions={
          <Button asChild>
            <Link to="/onboarding">Choose a starter</Link>
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
        <p role="status" className="text-12">
          Loading saved setups…
        </p>
      )}
      {setups.error && (
        <ErrorBanner
          message="Could not load your saved setups."
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
            <p className="text-11 text-muted-foreground">
              Revision {setup.revision} ·{" "}
              {setup.template === "restaurant-starter"
                ? "Restaurant starter"
                : "Telegram assistant"}
            </p>
            <Button asChild>
              <Link
                to="/onboarding/setups/$setupId"
                params={{ setupId: setup.id }}
              >
                Resume setup
              </Link>
            </Button>
          </section>
        ))}
      </div>
      {setups.data?.length === 0 && !template && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
          Your saved setups will appear here. Choose a starter to begin.
        </p>
      )}
      {actor && <AnonymousDrafts key={actor} />}
      {actor && <ArchivedSetups key={actor} />}
      <OwnedOnboardingBindings />
    </div>
  );
}
