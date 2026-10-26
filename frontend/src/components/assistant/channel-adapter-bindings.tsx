import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
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
  const actor = useAuthStore((s) => s.user?.id);
  return <OwnedAdapterBindings key={actor ?? "signed-out"} />;
}
function OwnedAdapterBindings() {
  const query = useChannelAdapterBindings();
  const catalog = useChannelAdapterCatalog();
  const [adding, setAdding] = useState(false);
  const profiles =
    catalog.data?.filter((d) => d.setup.agent === "nyxbot_specialist") ?? [];
  return (
    <section
      aria-label="Phone and speech connections"
      className="space-y-4 text-12"
    >
      <p>
        Answer calls and SMS with a scoped NyxBot specialist. Each caller gets a
        separate guest conversation. For a Notion FAQ, grant the specialist your
        Notion connection and set its Guest access to Read in the specialist’s
        permissions.
      </p>
      <p className="text-muted-foreground">
        The carrier handles the phone connection, the speech provider handles
        audio, and NyxBot runs the agent and its permitted tools. Browser voice
        permissions do not apply to callers.
      </p>
      {query.isPending && <p role="status">Loading connections…</p>}
      {(query.error || catalog.error) && (
        <ErrorBanner
          message="Could not load phone and speech connections."
          onRetry={() => {
            void query.refetch();
            void catalog.refetch();
          }}
        />
      )}
      {query.isSuccess && query.data?.length === 0 && (
        <p className="rounded-xl border border-dashed border-border p-4 text-muted-foreground">
          No phone or speech connections yet.
        </p>
      )}
      {query.data?.map((b) => (
        <BindingCard
          key={b.id}
          binding={b}
          descriptor={profiles.find((d) => d.adapter_id === b.adapter_id)}
        />
      ))}
      {query.hasNextPage && (
        <Button
          variant="outline"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          Load more connections
        </Button>
      )}
      <Button
        variant="outline"
        onClick={() => setAdding(true)}
        disabled={profiles.length === 0}
      >
        Add phone or speech connection
      </Button>
      <Dialog open={adding} onOpenChange={setAdding}>
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Connect a specialist</DialogTitle>
            <DialogDescription>
              NyxID verifies the provider account and resources before saving.
              This does not send a message or place a call.
            </DialogDescription>
          </DialogHeader>
          {adding && (
            <SetupForm descriptors={profiles} onDone={() => setAdding(false)} />
          )}
        </DialogContent>
      </Dialog>
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
    <article className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="font-medium">{b.label}</p>
          <p className="text-muted-foreground">
            {names[b.adapter_id] ?? b.adapter_id} ·{" "}
            {b.phone_number || "External speech"} ·{" "}
            {b.credential.kind === "platform"
              ? "Platform account"
              : "Your connection"}
          </p>
        </div>
        <span className="text-11">
          {!b.enabled
            ? "Disabled"
            : b.availability.available
              ? "Ready"
              : "Setup needed"}
        </span>
      </div>
      {b.availability.reason && (
        <p role="status">
          {adapterReason[b.availability.reason] ?? b.availability.reason}
        </p>
      )}
      <p className="text-muted-foreground">
        {b.adapter_id.startsWith("elevenlabs.")
          ? "Create a session for each ElevenLabs conversation and pass its extra_body from your client."
          : "Set the provider application’s inbound webhook and terminal/status callback to this URL (POST). Keep provider recording disabled unless separately authorized."}
      </p>
      <Input
        aria-label={`${b.label} webhook URL`}
        value={b.webhook_url}
        readOnly
      />
      <details open={detail} onToggle={(e) => setDetail(e.currentTarget.open)}>
        <summary className="cursor-pointer">Capabilities and readiness</summary>
        <div className="mt-3 space-y-2">
          <dl className="grid grid-cols-2 gap-2">
            <dt>Instructions refresh</dt>
            <dd>
              {operationName(d?.instructions.nyxbot_refresh ?? "unknown")}
            </dd>
            <dt>Delivery evidence</dt>
            <dd>
              {operationName(
                d?.messaging.outbound_text
                  ? d.messaging.delivery
                  : (d?.conversation.playback_evidence ?? "unknown"),
              )}
            </dd>
            <dt>SMS limit</dt>
            <dd>
              {d?.messaging.max_text_bytes
                ? `${d.messaging.max_text_bytes} bytes; multipart billed per provider part`
                : "Not applicable"}
            </dd>
          </dl>
          <ul className="space-y-1">
            {b.availability.operations.map((o) => (
              <li key={o.operation}>
                <span className="capitalize">{operationName(o.operation)}</span>
                :{" "}
                {o.available
                  ? "Ready"
                  : o.implemented
                    ? "Unavailable"
                    : "Unsupported"}
                {o.implemented && o.reason
                  ? ` — ${adapterReason[o.reason] ?? o.reason}`
                  : ""}
              </li>
            ))}
          </ul>
          <p className="text-muted-foreground">
            Provider acceptance does not prove delivery or that a caller heard a
            reply. Interrupted or uncertain effects are not automatically
            replayed.
          </p>
          {d?.setup.documentation && (
            <a
              className="underline"
              href={d.setup.documentation}
              target="_blank"
              rel="noreferrer"
            >
              Provider setup documentation
            </a>
          )}
        </div>
      </details>
      {error && <ErrorBanner message={error} />}
      <div className="flex flex-wrap justify-end gap-2">
        <Button
          variant="outline"
          disabled={actions.verify.isPending}
          onClick={() => void act(() => actions.verify.mutateAsync(b.id))}
        >
          {b.enabled ? "Verify again" : "Verify and enable"}
        </Button>
        {!b.enabled && (
          <Button
            variant="outline"
            disabled={actions.remove.isPending}
            onClick={() => void act(() => actions.remove.mutateAsync(b.id))}
          >
            Remove connection
          </Button>
        )}
        {b.enabled && (
          <Button
            variant="outline"
            disabled={actions.disable.isPending}
            onClick={() => void act(() => actions.disable.mutateAsync(b.id))}
          >
            Disable
          </Button>
        )}
        {b.enabled && b.adapter_id === "elevenlabs.custom_llm" && (
          <>
            <Button
              variant="outline"
              onClick={() =>
                void act(async () => {
                  const setup = await actions.customSetup.mutateAsync(b.id);
                  setSecret(JSON.stringify(setup, null, 2));
                  actions.customSetup.reset();
                })
              }
            >
              Show Custom LLM setup
            </Button>
            <Button
              variant="outline"
              onClick={() =>
                void act(async () => {
                  const session = await actions.customSession.mutateAsync(b.id);
                  setSessionId(session.session_id);
                  setSecret(JSON.stringify(session, null, 2));
                  actions.customSession.reset();
                })
              }
            >
              Create guest session
            </Button>
          </>
        )}
        {sessionId && (
          <Button
            variant="outline"
            onClick={() =>
              void act(async () => {
                await actions.endSession.mutateAsync({ id: b.id, sessionId });
                setSessionId(undefined);
                setSecret(undefined);
              })
            }
          >
            End guest session
          </Button>
        )}
      </div>
      <Dialog
        open={!!secret}
        onOpenChange={(open) => {
          if (!open) setSecret(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>ElevenLabs connection values</DialogTitle>
            <DialogDescription>
              These values authorize this connection. Copy them into your
              provider/client configuration. They disappear when this dialog
              closes.
            </DialogDescription>
          </DialogHeader>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-overlay p-3 text-11">
            {secret}
          </pre>
        </DialogContent>
      </Dialog>
    </article>
  );
}
function SetupForm({
  descriptors,
  onDone,
}: {
  descriptors: AdapterDescriptor[];
  onDone: () => void;
}) {
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
      <form onSubmit={form.handleSubmit(save)} className="space-y-4">
        <FormField
          control={form.control}
          name="adapter_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Profile</FormLabel>
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
              <FormLabel>Connection name</FormLabel>
              <FormControl>
                <Input {...field} placeholder="Support FAQ" />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="agent_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Specialist</FormLabel>
              <Select value={field.value} onValueChange={field.onChange}>
                <FormControl>
                  <SelectTrigger>
                    <SelectValue placeholder="Choose your specialist" />
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
              <p className="text-11 text-muted-foreground">
                Create a specialist in NyxBot’s Agents view first. For FAQ
                access, grant Notion and choose Guest access Read. The main
                NyxBot’s guest turns cannot use tools.
              </p>
              <FormMessage />
            </FormItem>
          )}
        />
        <div className="space-y-2">
          <label
            className="text-12 font-medium"
            htmlFor="adapter-credential-mode"
          >
            Credentials
          </label>
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
                    ? "Your service connection"
                    : "Explicit platform account"}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {descriptor?.setup.credential_modes
            .filter((m) => !m.implemented)
            .map((m) => (
              <p key={m.kind} className="text-11 text-muted-foreground">
                {adapterReason[m.prerequisite ?? ""] ?? m.prerequisite}
              </p>
            ))}
        </div>
        <div className="space-y-2">
          <label className="text-12 font-medium" htmlFor="adapter-connection">
            {mode === "connection" ? "Service connection" : "Platform service"}
          </label>
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
              <SelectValue placeholder="Choose an available service" />
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
            <p className="text-11 text-muted-foreground">
              Connect {provider} under Services first. Platform mode also
              requires an administrator’s allocation and configured billing.
            </p>
          )}
        </div>
        {(
          [
            ...(custom ? [] : ["account_id", "phone_number", "application_id"]),
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
                <FormLabel>Telnyx Ed25519 public key</FormLabel>
                <FormControl>
                  <Input {...field} value={field.value ?? ""} />
                </FormControl>
                <p className="text-11 text-muted-foreground">
                  Copy the public key from this account’s Mission Control.
                  Platform verification keys are managed by an administrator.
                </p>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
        <p className="text-11 text-muted-foreground">
          Find provider number/application IDs in the carrier console. Assign
          the number to that application before verification. Voice:{" "}
          {form.watch("voice")} · en-US. Twilio uses the pinned Google voice and
          a provider call time limit.
        </p>
        {error && <ErrorBanner message={error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onDone}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={
              !form.formState.isDirty ||
              !form.formState.isValid ||
              actions.create.isPending
            }
          >
            {actions.create.isPending ? "Verifying…" : "Verify and connect"}
          </Button>
        </div>
      </form>
    </Form>
  );
}
