import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { creditUnit } from "@/lib/platform-onboarding";
import { useAuthStore } from "@/stores/auth-store";
import { useState } from "react";
import { useParams } from "@tanstack/react-router";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useOwnedBindings,
  usePlatformSetup,
  useSetupMutation,
  useSetupApproval,
  useSetupProvisioning,
} from "@/hooks/use-platform-onboarding";
import {
  setupFactsSchema,
  setupLimitsSchema,
  setupStatus,
  type PlatformSetup,
  type SetupApproval,
} from "@/schemas/platform-onboarding";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";

const editSchema = z.object({
  facts: setupFactsSchema,
  limits: setupLimitsSchema,
});
const fieldLabels = {
  name: "Name",
  timezone: "Timezone",
  hours: "Opening hours",
  address: "Address",
  menu_url: "Menu link",
  reservation_url: "Reservation request link",
  contact: "Staff contact",
  policies: "Policies",
  assistant_instructions: "Public assistant instructions",
} as const;
export function SetupStatusBadge({ setup }: { setup: PlatformSetup }) {
  useUiTranslation();
  const status = setup.effective_state ?? setup.state;
  return (
    <Badge variant={status === "active" ? "success" : "secondary"}>
      {setupStatus[status]}
    </Badge>
  );
}

export function ProvisioningControls({ setup }: { setup: PlatformSetup }) {
  useUiTranslation();
  const needsReview = ["awaiting_approval", "awaiting_child_review"].includes(
    setup.state,
  );
  const approval = useSetupApproval(setup.id, needsReview);
  const mutation = useSetupProvisioning(setup.id);
  const [review, setReview] = useState<SetupApproval | null>(null);
  const actor = useAuthStore((state) => state.user?.id);
  const [launch, setLaunch] = useState<{
    url: string;
    actor?: string;
    revision: number;
    state: string;
  } | null>(null);
  const launchUrl =
    launch !== null &&
    actor !== undefined &&
    launch.actor === actor &&
    launch.revision === setup.revision &&
    launch.state === setup.state
      ? launch.url
      : null;
  const currentReview =
    !!review &&
    approval.data?.digest === review.digest &&
    approval.data.reviewed_revision === review.reviewed_revision &&
    approval.data.request_revision === review.request_revision &&
    setup.revision === review.reviewed_revision;
  if (
    ![
      "awaiting_approval",
      "awaiting_child_review",
      "provisioning",
      "ready_for_test",
      "active",
    ].includes(setup.state)
  )
    return null;
  return (
    <DetailSection title={translateUi("Provisioning and activation")}>
      <div className="space-y-3 p-4 text-14">
        {approval.error && <ErrorBanner message={approval.error.message} />}
        {mutation.error && <ErrorBanner message={mutation.error.message} />}
        {needsReview && approval.data && (
          <>
            <div className="space-y-2 rounded-lg border p-3">
              <p>{translateUi("Saved approval snapshot · revision")}{" "}
                {approval.data.reviewed_revision}
              </p>
              <dl className="space-y-2">
                {Object.entries(approval.data.setup.facts).map(
                  ([name, value]) => (
                    <div key={name}>
                      <dt className="font-medium">
                        {name.replaceAll("_", " ")}
                      </dt>
                      <dd className="whitespace-pre-wrap break-words">
                        {Array.isArray(value)
                          ? value
                              .map(
                                (entry) => `${entry.question}: ${entry.answer}`,
                              )
                              .join("\n") || translateUi("None")
                          : value || translateUi("None")}
                      </dd>
                    </div>
                  ),
                )}
              </dl>
              <p>{translateUi("Daily cap ")}{approval.data.setup.limits.daily_credits}{" "}
                {creditUnit(approval.data.setup.limits.daily_credits)};{" "}
                {approval.data.setup.limits.turns_per_hour}{translateUi(" turns/hour;")}{" "}
                {approval.data.setup.limits.concurrent_turns}{translateUi(" concurrent;")}{" "}
                {approval.data.setup.limits.max_input_chars}{translateUi(" input characters;")}{" "}
                {approval.data.setup.limits.max_output_tokens}{translateUi(" output tokens.")}</p>
              <p>{translateUi("Specialist:")}{" "}
                {approval.data.specialist_name ??
                  approval.data.setup.facts.name}
                {translateUi(". Customer payer:")}{" "}
                {approval.data.funding_owner_name ?? translateUi("Your account")}{translateUi(" (personal wallet). Model service:")}{" "}
                {approval.data.model_service_name ??
                  translateUi("Configured platform inference service")}
                .
              </p>
              <details className="break-all">
                <summary>{translateUi("Exact resource identifiers")}</summary>
                <p>{translateUi("Specialist: ")}{approval.data.agent_id}{translateUi(". Creation request:")}{" "}
                  {approval.data.creation_request_id}{translateUi(". Inference service:")}{" "}
                  {approval.data.model_service_id}{translateUi(". Customer payer:")}{" "}
                  {approval.data.funding_owner_user_id}.
                </p>
              </details>
              <p>{translateUi("Creates or resumes only your reviewed specialist and Telegram installation. No connector grants, machine access, private account context or account management are authorized.")}</p>
            </div>
            <Button
              disabled={approval.isFetching}
              onClick={() => {
                setReview(null);
                void approval.refetch();
              }}
            >{translateUi("Refresh approval snapshot")}</Button>

            <p>{translateUi("Model: ")}{approval.data.model_id}{translateUi(". Your account funds customer replies. No connectors, private account context or machine access are enabled. ")}</p>
            {approval.data.child_bot_id !== null && (
              <p>{translateUi("Telegram bot: @")}{approval.data.child_username ?? translateUi("unknown")} (
                {approval.data.child_bot_id}{translateUi("), management revision")}{" "}
                {approval.data.manager_revision}.
              </p>
            )}
            <Label className="flex gap-2">
              <Checkbox
                checked={currentReview}
                onCheckedChange={(checked) =>
                  setReview(checked === true ? approval.data! : null)
                }
              />
              <span>{translateUi("I approve these exact saved facts, limits, model and bot identity.")}</span>
            </Label>
            <Button
              disabled={!actor || !currentReview || mutation.isPending}
              onClick={() => {
                if (review && currentReview)
                  mutation.mutate({
                    action:
                      setup.state === "awaiting_child_review"
                        ? "child-approve"
                        : "approve",
                    review,
                  });
              }}
            >{translateUi("Approve reviewed setup")}</Button>
          </>
        )}
        {setup.state === "provisioning" && (
          <>
            <p>{translateUi("Open the manager chat to create your Telegram bot. Return here to review the created identity before registration proceeds.")}</p>
            <Button
              disabled={!actor || mutation.isPending}
              onClick={() =>
                mutation.mutate(
                  { action: "launch" },
                  {
                    onSuccess: (result) => {
                      if ("url" in result)
                        setLaunch({
                          url: result.url,
                          actor,
                          revision: setup.revision,
                          state: setup.state,
                        });
                    },
                  },
                )
              }
            >{translateUi("Prepare Telegram creation link")}</Button>
            {launchUrl && (
              <a
                href={launchUrl}
                rel="noreferrer"
                className="block underline"
                data-sensitive
              >{translateUi("Open Telegram manager")}</a>
            )}
          </>
        )}
        {setup.state === "ready_for_test" && (
          <>
            <p>{translateUi("Ask an independent customer to send /test to the child bot, then reply to its test message with the challenge it supplies. Your owner chat cannot establish customer delivery.")}</p>
            <Button
              disabled={!actor || !setup.delivery_tested || mutation.isPending}
              onClick={() =>
                mutation.mutate({
                  action: "activate",
                  revision: setup.revision,
                })
              }
            >{translateUi("Activate customer replies")}</Button>
          </>
        )}
        {setup.state === "active" && (
          <p>{translateUi("Active installations do not expire with draft retention or onboarding-link expiry. Pause, unlink, ownership changes or platform changes revoke their approved authority.")}</p>
        )}
      </div>
    </DetailSection>
  );
}

function SetupEditor({ setup }: { setup: PlatformSetup }) {
  useUiTranslation();
  const mutation = useSetupMutation(setup.id);
  const bindings = useOwnedBindings();
  const [bindingId, setBindingId] = useState("");
  const form = useAppForm({
    resolver: zodResolver(editSchema),
    defaultValues: { facts: setup.facts, limits: setup.limits },
  });
  const [revision, setRevision] = useState(setup.revision);
  const [reviewed, setReviewed] = useState(false);
  const [cancelReview, setCancelReview] = useState(false);
  const stale = setup.revision !== revision;
  const terminal = ["cancelled", "expired"].includes(setup.state);
  const fields =
    setup.template === "restaurant-starter"
      ? ([
          "name",
          "timezone",
          "hours",
          "address",
          "menu_url",
          "reservation_url",
          "contact",
          "policies",
        ] as const)
      : (["name", "timezone", "assistant_instructions"] as const);
  const accept = (saved: PlatformSetup) => {
    form.reset({ facts: saved.facts, limits: saved.limits });
    setRevision(saved.revision);
    setReviewed(false);
  };
  const action = (action: "review" | "publish" | "pause" | "cancel") =>
    mutation.mutate({ action, revision }, { onSuccess: accept });
  return (
    <div className="space-y-6">
      {stale && (
        <div className="space-y-2 rounded-lg border border-border p-4 text-14">
          <p>{translateUi("A newer revision is available. Reload it before saving or reviewing.")}</p>
          <Button onClick={() => accept(setup)}>{translateUi("Reload latest revision")}</Button>
        </div>
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      <DetailSection title={translateUi("Draft answers")}>
        <Form {...form}>
          <form
            className="space-y-4 p-4"
            onSubmit={form.handleSubmit((values) =>
              mutation.mutate(
                { action: "edit", revision, ...values },
                { onSuccess: accept },
              ),
            )}
          >
            <fieldset
              disabled={terminal || mutation.isPending || stale}
              className="space-y-4"
            >
              {fields.map((name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={`facts.${name}`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{fieldLabels[name]}</FormLabel>
                      <FormControl>
                        {[
                          "hours",
                          "policies",
                          "assistant_instructions",
                        ].includes(name) ? (
                          <Textarea {...field} value={field.value ?? ""} />
                        ) : (
                          <Input
                            {...field}
                            value={field.value ?? ""}
                            onChange={(event) =>
                              field.onChange(
                                name.endsWith("_url") && !event.target.value
                                  ? null
                                  : event.target.value,
                              )
                            }
                          />
                        )}
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
              {setup.template === "restaurant-starter" && (
                <div className="space-y-3">
                  <h3 className="type-card-title">{translateUi("Frequently asked questions")}</h3>
                  {form.watch("facts.faq").map((_, index) => (
                    <div
                      key={index}
                      className="space-y-2 rounded-lg border border-border p-3"
                    >
                      <FormField
                        control={form.control}
                        name={`facts.faq.${index}.question`}
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>{translateUi("Question ")}{index + 1}</FormLabel>
                            <FormControl>
                              <Input {...field} />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <FormField
                        control={form.control}
                        name={`facts.faq.${index}.answer`}
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>{translateUi("Answer ")}{index + 1}</FormLabel>
                            <FormControl>
                              <Textarea {...field} />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <Button
                        type="button"
                        onClick={() =>
                          form.setValue(
                            "facts.faq",
                            form
                              .getValues("facts.faq")
                              .filter((_, at) => at !== index),
                          )
                        }
                      >{translateUi("Remove question")}</Button>
                    </div>
                  ))}
                  <Button
                    type="button"
                    disabled={form.watch("facts.faq").length >= 20}
                    onClick={() =>
                      form.setValue("facts.faq", [
                        ...form.getValues("facts.faq"),
                        { question: "", answer: "" },
                      ])
                    }
                  >{translateUi("Add question")}</Button>
                </div>
              )}
              <h3 className="type-card-title">{translateUi("Runtime limits")}</h3>
              <p className="text-14 text-muted-foreground">{translateUi("These caps are reviewed separately from activation. Public restaurant requests require staff confirmation.")}</p>
              <div className="grid gap-4 sm:grid-cols-2">
                {(
                  [
                    "daily_credits",
                    "turns_per_hour",
                    "concurrent_turns",
                    "max_input_chars",
                    "max_output_tokens",
                  ] as const
                ).map((name) => (
                  <FormField
                    key={name}
                    control={form.control}
                    name={`limits.${name}`}
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>
                          {
                            {
                              daily_credits: "Daily credit cap",
                              turns_per_hour: "Turns per hour",
                              concurrent_turns: "Concurrent turns",
                              max_input_chars: "Input character cap",
                              max_output_tokens: "Output token cap",
                            }[name]
                          }
                        </FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            type={name === "daily_credits" ? "text" : "number"}
                            onChange={(event) =>
                              field.onChange(
                                name === "daily_credits"
                                  ? event.target.value
                                  : Number(event.target.value),
                              )
                            }
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ))}
              </div>
              <FormSubmitErrors />
              <div className="flex justify-end gap-2">
                <Button
                  type="button"
                  disabled={!form.formState.isDirty}
                  onClick={() =>
                    form.reset({ facts: setup.facts, limits: setup.limits })
                  }
                >{translateUi("Discard edits")}</Button>
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!form.formState.isDirty}
                  isLoading={mutation.isPending}
                >{translateUi("Save draft")}</Button>
              </div>
            </fieldset>
          </form>
        </Form>
      </DetailSection>
      <DetailSection title={translateUi("Review the saved revision")}>
        <div className="space-y-3 p-4 text-14">
          <p>{translateUi("Revision ")}{setup.revision} · {setupStatus[setup.state]} ·{" "}
            {setup.approved ? translateUi("Approval recorded") : translateUi("Approval required")}
          </p>
          <p className="text-muted-foreground">{translateUi("Saving changes invalidates earlier approval and customer testing. Login and editing never approve resource creation.")}</p>
          <div className="space-y-2 rounded-lg border border-border p-3">
            {fields.map((name) => (
              <p key={name} className="break-words">
                <strong>{fieldLabels[name]}:</strong>{" "}
                {setup.facts[name] || translateUi("Not provided")}
              </p>
            ))}
            {setup.facts.faq.map((faq, index) => (
              <div key={index}>
                <p className="font-semibold">{faq.question}</p>
                <p className="whitespace-pre-wrap">{faq.answer}</p>
              </div>
            ))}
          </div>
          <p>{translateUi("Daily cap: ")}{setup.limits.daily_credits}{" "}
            {creditUnit(setup.limits.daily_credits)} ·{" "}
            {setup.limits.turns_per_hour}{translateUi(" turns/hour ·")}{" "}
            {setup.limits.concurrent_turns}{translateUi(" concurrent ·")}{" "}
            {setup.limits.max_input_chars}{translateUi(" input characters ·")}{" "}
            {setup.limits.max_output_tokens}{translateUi(" output tokens.")}</p>
          <p>{translateUi("No connector grants.")}{" "}
            {setup.delivery_tested
              ? translateUi("Customer receipt verified.")
              : translateUi("Customer receipt has not been verified.")}
          </p>
          <Label className="flex items-start gap-2">
            <Checkbox
              checked={reviewed}
              onCheckedChange={(value) => setReviewed(value === true)}
              disabled={terminal || stale || form.formState.isDirty}
            />
            <span>{translateUi("I reviewed every saved answer and limit in this revision.")}</span>
          </Label>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              disabled={
                !reviewed ||
                terminal ||
                stale ||
                mutation.isPending ||
                form.formState.isDirty
              }
              onClick={() => action("review")}
            >{translateUi("Prepare approval review")}</Button>
            <Button
              disabled={
                !reviewed ||
                terminal ||
                stale ||
                mutation.isPending ||
                form.formState.isDirty
              }
              onClick={() => action("publish")}
            >{translateUi("Save reviewed public snapshot")}</Button>
          </div>
          <p className="text-muted-foreground">{translateUi("A reviewed snapshot stays private until the installation passes setup, customer testing and activation.")}</p>
        </div>
      </DetailSection>
      <ProvisioningControls
        key={`${setup.id}:${setup.revision}:${setup.state}`}
        setup={setup}
      />
      {!terminal && (
        <DetailSection title={translateUi("Link this setup to Telegram")}>
          <div className="space-y-3 p-4 text-14">
            <p>{translateUi("Only the selected manager chat receives progress for this setup. Your other account setups remain private.")}</p>
            {bindings.error && (
              <ErrorBanner message={translateUi("Could not load your bindings.")} />
            )}
            <Label className="block space-y-2">{translateUi("Your linked private Telegram account")}<select
                className="h-8 w-full rounded-lg border border-input bg-background px-3 text-14"
                value={bindingId}
                onChange={(event) => setBindingId(event.target.value)}
              >
                <option value="">{translateUi("Choose a binding")}</option>
                {bindings.data?.map((binding) => (
                  <option key={binding.id} value={binding.id}>{translateUi("Manager ")}{binding.manager_bot_id}{translateUi(" · sender")}{" "}
                    {binding.telegram_sender_id}
                  </option>
                ))}
              </select>
            </Label>
            <Button
              disabled={
                !bindingId ||
                stale ||
                form.formState.isDirty ||
                mutation.isPending
              }
              onClick={() => {
                const binding = bindings.data?.find(
                  (row) => row.id === bindingId,
                );
                if (binding)
                  mutation.mutate(
                    {
                      action: "binding",
                      revision,
                      binding_id: binding.id,
                      binding_generation: binding.generation,
                    },
                    {
                      onSuccess: (saved) => {
                        accept(saved);
                        setBindingId("");
                      },
                    },
                  );
              }}
            >{translateUi("Link selected Telegram account")}</Button>
          </div>
        </DetailSection>
      )}
      {!terminal && (
        <DetailSection title={translateUi("Pause or cancel")}>
          <div className="space-y-3 p-4 text-14">
            <p>{translateUi("Pause blocks new work. Cancellation retains your created resources and invalidates this setup's approval.")}</p>
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={stale || mutation.isPending}
                onClick={() => action("pause")}
              >{translateUi("Pause setup")}</Button>
              <Button
                disabled={stale || mutation.isPending}
                onClick={() => setCancelReview(true)}
              >{translateUi("Review cancellation")}</Button>
            </div>
            {cancelReview && (
              <div className="space-y-2">
                <p>{translateUi("Cancel this setup? Existing Telegram bots and client resources remain yours.")}</p>
                <Button
                  variant="destructive"
                  isLoading={mutation.isPending}
                  onClick={() => action("cancel")}
                >{translateUi("Confirm cancellation")}</Button>
              </div>
            )}
          </div>
        </DetailSection>
      )}
    </div>
  );
}
export function PlatformSetupDetailPage() {
  useUiTranslation();
  const params = useParams({ strict: false }) as { setupId?: string };
  const actor = useAuthStore((s) => s.user?.id);
  const validId = z.uuid().safeParse(params.setupId).success;
  const query = usePlatformSetup(params.setupId ?? "");
  useBreadcrumbLabel(query.data?.facts.name ?? "Bot setup");
  return (
    <div className="space-y-6">
      <PageHeader
        title={query.data?.facts.name ?? translateUi("Bot setup")}
        description={translateUi("Your saved answers, review and runtime limits.")}
        actions={
          <div className="flex items-center gap-2">
            <Button
              disabled={!validId || query.isFetching}
              onClick={() => void query.refetch()}
            >{translateUi("Refresh status")}</Button>
            {query.data && <SetupStatusBadge setup={query.data} />}
          </div>
        }
      />
      {!validId && (
        <ErrorBanner message={translateUi("This setup is unavailable. Choose a saved setup from your list.")} />
      )}
      {query.data?.runtime_notice && (
        <p role="status" className="text-14">
          {query.data.runtime_notice}
        </p>
      )}
      {validId && query.isPending && (
        <p role="status" className="text-14">{translateUi("Loading setup…")}</p>
      )}
      {query.error && (
        <ErrorBanner
          message={translateUi("This setup is unavailable to your account.")}
          onRetry={() => void query.refetch()}
        />
      )}
      {query.data && (
        <SetupEditor key={`${actor}:${query.data.id}`} setup={query.data} />
      )}
    </div>
  );
}
