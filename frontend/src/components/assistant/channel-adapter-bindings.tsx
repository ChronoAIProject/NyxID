import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  GroupedInput as Input,
  GroupedSelectTrigger as SelectTrigger,
  GroupedList,
  GroupedRow,
  GroupedDisclosure,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedLabel as FormLabel,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import {
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import { Form, FormControl, FormField, useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useAuthStore } from "@/stores/auth-store";
import { useKeys } from "@/hooks/use-keys";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import {
  useChannelAdapterActions,
  useChannelAdapterBindings,
  useChannelAdapterCatalog,
} from "@/hooks/use-channel-adapters";
import {
  adapterSetupSchema,
  adapterReason,
  type AdapterBinding,
  type AdapterDescriptor,
  type AdapterSetup,
} from "@/schemas/channel-adapters";

const names: Record<string, string> = {
  "telnyx.conversation_relay": "Telnyx Conversation Relay",
  "telnyx.messaging": "Telnyx SMS",
  "twilio.conversation_relay": "Twilio ConversationRelay",
  "twilio.messaging": "Twilio SMS",
  "elevenlabs.custom_llm": "ElevenLabs Custom LLM",
};
const operationName = (name: string) => name.replaceAll("_", " ");
export function ChannelAdapterBindings() {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  return <OwnedAdapterBindings key={actor ?? "signed-out"} />;
}
function OwnedAdapterBindings() {
  useUiTranslation();
  const query = useChannelAdapterBindings();
  const catalog = useChannelAdapterCatalog();
  const [adding, setAdding] = useState(false);
  const profiles =
    catalog.data?.filter((d) => d.setup.agent === "nyxbot_specialist") ?? [];
  return (
    <section className="space-y-4 text-14">
      <GroupedList
        title={translateUi("Phone and speech connections")}
        footer={
          <>
            <p>{translateUi("Answer calls and SMS with a scoped NyxBot specialist. Each caller gets a separate guest conversation. For a Notion FAQ, grant the specialist your Notion connection and set its Guest access to Read in the specialist’s permissions.")}</p>
            <p>{translateUi("The carrier handles the phone connection, the speech provider handles audio, and NyxBot runs the agent and its permitted tools. Browser voice permissions do not apply to callers.")}</p>
            {(query.error || catalog.error) && (
              <ErrorBanner
                message={translateUi("Could not load phone and speech connections.")}
                onRetry={() => {
                  void query.refetch();
                  void catalog.refetch();
                }}
              />
            )}
          </>
        }
      >
        {query.isPending && (
          <GroupedRow title={translateUi("Loading connections…")} role="status" />
        )}
        {query.isSuccess && query.data?.length === 0 && (
          <GroupedRow title={translateUi("No phone or speech connections yet.")} />
        )}
        <GroupedRow asChild interactive>
          <button
            type="button"
            onClick={() => setAdding(true)}
            disabled={profiles.length === 0}
          >{translateUi("Add phone or speech connection")}</button>
        </GroupedRow>
        {query.hasNextPage && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={query.isFetchingNextPage}
              onClick={() => void query.fetchNextPage()}
            >{translateUi("Load more connections")}</button>
          </GroupedRow>
        )}
      </GroupedList>
      {query.data?.map((b) => (
        <BindingCard
          key={b.id}
          binding={b}
          descriptor={profiles.find((d) => d.adapter_id === b.adapter_id)}
        />
      ))}
      <GroupedDialog open={adding} onOpenChange={setAdding}>
        <GroupedDialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{translateUi("Connect a specialist")}</DialogTitle>
            <DialogDescription>{translateUi("NyxID verifies the provider account and resources before saving. This does not send a message or place a call.")}</DialogDescription>
          </DialogHeader>
          {adding && (
            <SetupForm descriptors={profiles} onDone={() => setAdding(false)} />
          )}
        </GroupedDialogContent>
      </GroupedDialog>
    </section>
  );
}
function BindingCard({
  binding: b,
  descriptor: d,
}: {
  binding: AdapterBinding;
  descriptor?: AdapterDescriptor;
}) {
  useUiTranslation();
  const actions = useChannelAdapterActions();
  const [detail, setDetail] = useState(false);
  const [secret, setSecret] = useState<string>();
  const [error, setError] = useState<string>();
  const [sessionId, setSessionId] = useState<string>();
  async function act(task: () => Promise<unknown>) {
    setError(undefined);
    try {
      await task();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Connection action failed.");
    }
  }
  return (
    <div className="space-y-6">
      <GroupedList
        title={b.label}
        footer={
          <>
            {b.availability.reason && (
              <p role="status">
                {adapterReason[b.availability.reason] ?? b.availability.reason}
              </p>
            )}
            <p>
              {b.adapter_id.startsWith("elevenlabs.")
                ? translateUi("Create a session for each ElevenLabs conversation and pass its extra_body from your client.")
                : translateUi("Set the provider application’s inbound webhook and terminal/status callback to this URL (POST). Keep provider recording disabled unless separately authorized.")}
            </p>
            {error && <ErrorBanner message={error} />}
          </>
        }
      >
        <GroupedRow
          title={names[b.adapter_id] ?? b.adapter_id}
          description={`${b.phone_number || "External speech"} · ${b.credential.kind === "platform" ? "Platform account" : "Your connection"}`}
          accessory={
            !b.enabled
              ? "Disabled"
              : b.availability.available
                ? "Ready"
                : "Setup needed"
          }
        />
        <GroupedRow asChild className="block space-y-0.5">
          <Label>
            <span className="text-text-tertiary">{translateUi("Webhook URL")}</span>
            <Input
              aria-label={translateUi("{{value1}} webhook URL", { value1: b.label })}
              value={b.webhook_url}
              readOnly
            />
          </Label>
        </GroupedRow>
        <GroupedDisclosure
          aria-expanded={detail}
          onClick={() => setDetail(!detail)}
        >{translateUi("Capabilities and readiness")}</GroupedDisclosure>

        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={actions.verify.isPending}
            onClick={() => void act(() => actions.verify.mutateAsync(b.id))}
          >
            {b.enabled ? translateUi("Verify again") : translateUi("Verify and enable")}
          </button>
        </GroupedRow>
        {!b.enabled && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              className="text-destructive"
              disabled={actions.remove.isPending}
              onClick={() => void act(() => actions.remove.mutateAsync(b.id))}
            >{translateUi("Remove connection")}</button>
          </GroupedRow>
        )}
        {b.enabled && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              className="text-destructive"
              disabled={actions.disable.isPending}
              onClick={() => void act(() => actions.disable.mutateAsync(b.id))}
            >{translateUi("Disable")}</button>
          </GroupedRow>
        )}
        {b.enabled && b.adapter_id === "elevenlabs.custom_llm" && (
          <>
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() =>
                  void act(async () => {
                    const setup = await actions.customSetup.mutateAsync(b.id);
                    setSecret(JSON.stringify(setup, null, 2));
                    actions.customSetup.reset();
                  })
                }
              >{translateUi("Show Custom LLM setup")}</button>
            </GroupedRow>
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() =>
                  void act(async () => {
                    const session = await actions.customSession.mutateAsync(b.id);
                    setSessionId(session.session_id);
                    setSecret(JSON.stringify(session, null, 2));
                    actions.customSession.reset();
                  })
                }
              >{translateUi("Create guest session")}</button>
            </GroupedRow>
          </>
        )}
        {sessionId && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              onClick={() =>
                void act(async () => {
                  await actions.endSession.mutateAsync({ id: b.id, sessionId });
                  setSessionId(undefined);
                  setSecret(undefined);
                })
              }
            >{translateUi("End guest session")}</button>
          </GroupedRow>
        )}
      </GroupedList>
      {detail && (
        <GroupedList
          title={translateUi("Capabilities and readiness")}
          footer={translateUi("Provider acceptance does not prove delivery or that a caller heard a reply. Interrupted or uncertain effects are not automatically replayed.")}
        >
          <GroupedRow
            title={translateUi("Instructions refresh")}
            accessory={operationName(
              d?.instructions.nyxbot_refresh ?? "unknown",
            )}
          />
          <GroupedRow
            title={translateUi("Delivery evidence")}
            accessory={operationName(
              d?.messaging.outbound_text
                ? d.messaging.delivery
                : (d?.conversation.playback_evidence ?? "unknown"),
            )}
          />
          <GroupedRow
            title={translateUi("SMS limit")}
            accessory={
              d?.messaging.max_text_bytes
                ? `${d.messaging.max_text_bytes} bytes; multipart billed per provider part`
                : "Not applicable"
            }
          />
          {b.availability.operations.map((o) => (
            <GroupedRow
              key={o.operation}
              title={
                <span className="capitalize">{operationName(o.operation)}</span>
              }
              description={
                o.implemented && o.reason
                  ? (adapterReason[o.reason] ?? o.reason)
                  : undefined
              }
              accessory={
                o.available
                  ? "Ready"
                  : o.implemented
                    ? "Unavailable"
                    : "Unsupported"
              }
            />
          ))}
          {d?.setup.documentation && (
            <GroupedRow asChild interactive>
              <a href={d.setup.documentation} target="_blank" rel="noreferrer">{translateUi("Provider setup documentation")}</a>
            </GroupedRow>
          )}
        </GroupedList>
      )}
      <Dialog
        open={!!secret}
        onOpenChange={(open) => {
          if (!open) setSecret(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("ElevenLabs connection values")}</DialogTitle>
            <DialogDescription>{translateUi("These values authorize this connection. Copy them into your provider/client configuration. They disappear when this dialog closes.")}</DialogDescription>
          </DialogHeader>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-overlay p-3 text-14">
            {secret}
          </pre>
        </DialogContent>
      </Dialog>
    </div>
  );
}
function SetupForm({
  descriptors,
  onDone,
}: {
  descriptors: AdapterDescriptor[];
  onDone: () => void;
}) {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  const agents = useNyxBotAgents();
  const keys = useKeys();
  const actions = useChannelAdapterActions();
  const [mode, setMode] = useState<"connection" | "platform">("connection");
  const [error, setError] = useState<string>();
  const form = useAppForm<AdapterSetup>({
    resolver: zodResolver(adapterSetupSchema),
    mode: "onChange",
    defaultValues: {
      adapter_id: descriptors[0]?.adapter_id ?? "telnyx.conversation_relay",
      contract_version: 1,
      label: "",
      agent_id: "",
      credential: { kind: "connection", user_service_id: "" },
      account_id: "",
      resource_id: "",
      phone_number: "",
      application_id: "",
      webhook_public_key: null,
      voice: "Telnyx.Ultra.Callie",
      language: "en-US",
    },
  });
  const selected = form.watch("adapter_id");
  const descriptor = descriptors.find((d) => d.adapter_id === selected);
  const provider = selected.split(".")[0];
  const custom = provider === "elevenlabs";
  const specialists =
    agents.data?.agents.filter(
      (a) =>
        a.kind === "specialist" &&
        a.status !== "destroyed" &&
        a.owner_kind !== "org" &&
        (!a.owner_id || a.owner_id === actor),
    ) ?? [];
  const candidates =
    keys.data?.filter(
      (k) =>
        k.catalog_service_id &&
        !k.node_id &&
        k.is_active &&
        (k.credential_source?.type !== "org" || k.credential_source.allowed) &&
        (k.catalog_service_slug?.includes(provider ?? "") ||
          k.endpoint_url?.includes(`api.${provider}.`)) &&
        (mode === "platform"
          ? k.platform_key_available
          : k.credential_binding !== "platform" && k.status === "active"),
    ) ?? [];
  const connections =
    mode === "platform"
      ? [...new Map(candidates.map((k) => [k.catalog_service_id, k])).values()]
      : candidates;
  const credential = form.watch("credential");
  const selectedConnection =
    credential.kind === "connection"
      ? credential.user_service_id
      : credential.catalog_service_id;
  function profile(id: string) {
    form.setValue("adapter_id", id);
    setMode("connection");
    form.setValue("credential", { kind: "connection", user_service_id: "" });
    form.setValue(
      "voice",
      id.startsWith("twilio.") ? "en-US-Standard-A" : "Telnyx.Ultra.Callie",
    );
    if (id.startsWith("elevenlabs.")) {
      form.setValue("account_id", "elevenlabs");
      form.setValue("application_id", "external-speech");
    }
  }
  async function save(v: AdapterSetup) {
    setError(undefined);
    try {
      await actions.create.mutateAsync(v);
      onDone();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Provider verification failed.",
      );
    }
  }
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(save)} className="space-y-6">
        <GroupedList title={translateUi("Connection")}>
          <FormField
            control={form.control}
            name="adapter_id"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel>{translateUi("Profile")}</FormLabel>
                <Select value={field.value} onValueChange={profile}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {descriptors.map((d) => (
                      <SelectItem key={d.adapter_id} value={d.adapter_id}>
                        {names[d.adapter_id] ?? d.adapter_id}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="label"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{translateUi("Connection name")}</FormLabel>
                <FormControl>
                  <Input {...field} placeholder={translateUi("Support FAQ")} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="agent_id"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel>{translateUi("Specialist")}</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue placeholder={translateUi("Choose your specialist")} />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {specialists.map((a) => (
                      <SelectItem key={a.id} value={a.id}>
                        {a.display_name ?? a.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <GroupedFooter>
                  <p>{translateUi("Create a specialist in NyxBot’s Agents view first. For FAQ access, grant Notion and choose Guest access Read. The main NyxBot’s guest turns cannot use tools.")}</p>
                </GroupedFooter>
                <FormMessage />
              </FormItem>
            )}
          />
          <GroupedRow>
            <Label
              className="min-w-0 flex-1 text-foreground"
              htmlFor="adapter-credential-mode"
            >{translateUi("Credentials")}</Label>
            <Select
              value={mode}
              onValueChange={(v) => {
                const next = v as typeof mode;
                setMode(next);
                form.setValue(
                  "credential",
                  next === "connection"
                    ? { kind: "connection", user_service_id: "" }
                    : { kind: "platform", catalog_service_id: "" },
                );
              }}
            >
              <SelectTrigger id="adapter-credential-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {descriptor?.setup.credential_modes.map((m) => (
                  <SelectItem
                    key={m.kind}
                    value={m.kind}
                    disabled={!m.implemented}
                  >
                    {m.kind === "connection"
                      ? translateUi("Your service connection")
                      : translateUi("Explicit platform account")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {descriptor?.setup.credential_modes
              .filter((m) => !m.implemented)
              .map((m) => (
                <GroupedFooter key={m.kind}>
                  <p>{adapterReason[m.prerequisite ?? ""] ?? m.prerequisite}</p>
                </GroupedFooter>
              ))}
          </GroupedRow>
          <GroupedRow>
            <Label
              className="min-w-0 flex-1 text-foreground"
              htmlFor="adapter-connection"
            >
              {mode === "connection"
                ? translateUi("Service connection")
                : translateUi("Platform service")}
            </Label>
            <Select
              value={selectedConnection}
              onValueChange={(id) =>
                form.setValue(
                  "credential",
                  mode === "connection"
                    ? { kind: "connection", user_service_id: id }
                    : { kind: "platform", catalog_service_id: id },
                )
              }
            >
              <SelectTrigger id="adapter-connection">
                <SelectValue placeholder={translateUi("Choose an available service")} />
              </SelectTrigger>
              <SelectContent>
                {connections.map((k) => (
                  <SelectItem
                    key={k.id}
                    value={mode === "connection" ? k.id : k.catalog_service_id!}
                  >
                    {k.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {connections.length === 0 && (
              <GroupedFooter>
                <p>{translateUi("Connect ")}{provider}{translateUi(" under Services first. Platform mode also requires an administrator’s allocation and configured billing.")}</p>
              </GroupedFooter>
            )}
          </GroupedRow>
          {(
            [
              ...(custom
                ? []
                : ["account_id", "phone_number", "application_id"]),
              "resource_id",
            ] as const
          ).map((name) => (
            <FormField
              key={name}
              control={form.control}
              name={
                name as
                  | "account_id"
                  | "phone_number"
                  | "application_id"
                  | "resource_id"
              }
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    {
                      {
                        account_id: "Provider account ID",
                        phone_number: "Phone number",
                        application_id:
                          "Provider application / messaging profile ID",
                        resource_id: custom
                          ? "ElevenLabs agent ID"
                          : "Provider number resource ID",
                      }[name]
                    }
                  </FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          ))}
          {provider === "telnyx" && mode === "connection" && (
            <FormField
              control={form.control}
              name="webhook_public_key"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Telnyx Ed25519 public key")}</FormLabel>
                  <FormControl>
                    <Input {...field} value={field.value ?? ""} />
                  </FormControl>
                  <GroupedFooter>
                    <p>{translateUi("Copy the public key from this account’s Mission Control. Platform verification keys are managed by an administrator.")}</p>
                  </GroupedFooter>
                  <FormMessage />
                </FormItem>
              )}
            />
          )}
          <GroupedFooter>
            <p>{translateUi("Find provider number/application IDs in the carrier console. Assign the number to that application before verification. Voice:")}{" "}
              {form.watch("voice")}{translateUi(" · en-US. Twilio uses the pinned Google voice and a provider call time limit.")}</p>
          </GroupedFooter>
          <GroupedFooter>
            {error && <ErrorBanner message={error} />}
          </GroupedFooter>
        </GroupedList>
        <GroupedDialogActions
          intent="create"
          label={translateUi("phone or speech connection")}
          dirty={form.formState.isDirty}
          pending={actions.create.isPending}
          disabled={!form.formState.isValid}
          save={form.handleSubmit(save)}
          discard={() => {
            form.reset();
            setMode("connection");
            setError(undefined);
          }}
          saveLabel={
            actions.create.isPending ? translateUi("Verifying…") : translateUi("Verify and connect")
          }
        />
      </form>
    </Form>
  );
}
