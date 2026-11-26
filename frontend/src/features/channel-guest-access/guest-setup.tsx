import { useId, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { useWatch } from "react-hook-form";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { FieldHelp, Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { NativeSelect } from "@/components/ui/native-select";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import {
  guestSetupQueryKey,
  useChannelGuestSetup,
  usePreviewGuestSetup,
  useGuestSetupPreference,
  usePrepareGuestSetup,
  useSaveGuestSetup,
} from "@/hooks/use-channel-guest-setup";
import { ApiError } from "@/lib/api-client";
import { channelPlatformName } from "@/lib/assistant/nyxbot-labels";
import { formatDateTime } from "@/lib/utils";
import {
  guestSetupFormSchema,
  type GuestAction as Action,
  type GuestDataRule as Rule,
  type GuestObservation,
  type GuestSetupConfiguration as Configuration,
  type GuestSetupSettings as Settings,
  type GuestSetupSource as Source,
  type GuestSetupPreview as Preview,
} from "@/schemas/channel-guest-access";
import type { GuestSkillSelection } from "@/schemas/channel-guest-skills";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { GuestSkillExposure } from "./guest-skill-exposure";
import { useGuestSkills } from "./guest-skills-query";

const EMPTY: Configuration = { audience: null, rules: [], declarations: [] };
const EXAMPLE_READ_RANGE = "Event_Config!A1:H24";
const EXAMPLE_APPEND_TAB = "Questions";

export function GuestSetupLink({ botId }: { readonly botId: string }) {
  useUiTranslation();
  const setup = useChannelGuestSetup(botId);
  if (!setup.data?.available && !setup.data?.activation_issue) return null;
  return (
    <Button asChild variant="outline" size="sm">
      <Link to="/channel-bots/$botId" params={{ botId }} hash="guest-experience">
        {translateUi("Guest experience")}
      </Link>
    </Button>
  );
}

export function GuestSetupEditor({ botId }: { readonly botId: string }) {
  return <SetupPanel key={botId} botId={botId} />;
}

function SetupPanel({ botId }: { readonly botId: string }) {
  const { t } = useUiTranslation();
  const client = useQueryClient();
  const setup = useChannelGuestSetup(botId);
  const [opened, setOpened] = useState(false);
  const staged = useChannelGuestSetup(botId, true, opened && setup.data?.available === false);
  const preference = useGuestSetupPreference(botId);
  const id = useId();
  const data = setup.data;
  if (!data) return null;
  const settings = data.available ? data.settings : opened ? staged.data?.settings : undefined;
  const setPreference = (enabled: boolean) => {
    preference.mutate({ enabled, revision: data.bot_revision }, {
      onSuccess: (response) => {
        client.setQueryData(guestSetupQueryKey(botId), response);
        void client.invalidateQueries({ queryKey: guestSetupQueryKey(botId, true) });
      },
      onError: () => void setup.refetch(),
    });
  };

  return (
    <section id="guest-experience" className="space-y-4 rounded-xl border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="space-y-1">
          <Label htmlFor={id}>{t("Use guest access v1")}</Label>
          <FieldHelp>
            {data.available ? t("Guest access v1 is active for this bot.") : t("This bot follows its ordinary channel settings. Guest access v1 applies when the platform rollout includes this bot.")}
          </FieldHelp>
          <FieldHelp>{t("Enrolled guest conversations accept text. Guest images and documents use ordinary channel settings outside the rollout.")}</FieldHelp>
        </div>
        <Switch id={id} checked={data.enabled} disabled={preference.isPending} onCheckedChange={setPreference} />
      </div>
      {data.activation_issue && <p role="status" className="type-secondary text-muted-foreground">
        {t("The prepared draft needs another review. Update it before retrying activation.")}
      </p>}
      {preference.error && <p role="alert" className="type-secondary text-destructive">{preference.error.message}</p>}
      {!data.available && !opened && <Button variant="outline" onClick={() => setOpened(true)}>{t("Prepare guest access")}</Button>}
      {!data.available && opened && !settings && (
        staged.isError
          ? <Button variant="outline" onClick={() => void staged.refetch()}>{t("Reload guest settings")}</Button>
          : <p role="status" className="type-secondary text-muted-foreground">{t("Loading guest settings…")}</p>
      )}
      {settings && <Editor key={data.available ? "active" : "staging"} botId={botId} settings={settings}
        staging={!data.available}
        onPrepared={() => {
          if (data.enabled && data.activation_issue) setPreference(true);
        }} />}
    </section>
  );
}

function actionLabel(action: Action) {
  switch (action) {
    case "create": return translateUi("Create");
    case "read": return translateUi("Read");
    case "update": return translateUi("Update");
    case "delete": return translateUi("Delete");
  }
}

function defaultFields(source: Source, action: Action) {
  return {
    read: action === "read" ? source.fields : [],
    write: action === "create" || action === "update" ? source.fields : [],
  };
}

function GuestChoices({
  guests, selected, onChange,
}: {
  readonly guests: GuestObservation[];
  readonly selected: string[];
  readonly onChange: (ids: string[]) => void;
}) {
  const ids = [...new Set([...guests.map((guest) => guest.id), ...selected])];
  return ids.length === 0 ? (
    <FieldHelp>{translateUi("Verified guests appear after they message this bot through a supported channel.")}</FieldHelp>
  ) : (
    <div className="grid gap-2">
      {ids.map((id) => (
        <Label key={id} className="flex items-center gap-2">
          <Checkbox checked={selected.includes(id)} onCheckedChange={(checked) =>
            onChange(checked ? [...selected, id] : selected.filter((value) => value !== id))} />
          {guestLabel(id, guests.find((guest) => guest.id === id))}
        </Label>
      ))}
    </div>
  );
}

function guestLabel(id: string, observation?: GuestObservation) {
  const shortId = id.slice(-12);
  if (!observation) return translateUi("Guest {{id}}", { id: shortId });
  return translateUi("{{platform}} guest {{id}} · Seen {{date}}", {
    platform: channelPlatformName(observation.platform),
    id: shortId,
    date: formatDateTime(observation.last_seen_at),
  });
}

function Editor({ botId, settings, staging, onPrepared }: {
  readonly botId: string;
  readonly settings: Settings;
  readonly staging: boolean;
  readonly onPrepared: () => void;
}) {
  useUiTranslation();
  const client = useQueryClient();
  const skills = useGuestSkills(botId);
  const id = useId();
  const [revision, setRevision] = useState(settings.revision);
  const form = useAppForm({
    resolver: zodResolver(guestSetupFormSchema),
    defaultValues: { configuration: settings.draft ?? settings.applied ?? EMPTY },
  });
  const configuration = useWatch({ control: form.control, name: "configuration" });
  const [checked, setChecked] = useState<Preview>();
  const preview = usePreviewGuestSetup(botId, staging);
  const save = useSaveGuestSetup(botId, staging);
  const prepare = usePrepareGuestSetup(botId);
  const [skillSelection, setSkillSelection] = useState<GuestSkillSelection>();
  const key = guestSetupQueryKey(botId, staging);
  const remoteChange = revision !== settings.revision;
  const reload = () => {
    form.reset({ configuration: settings.draft ?? settings.applied ?? EMPTY });
    setRevision(settings.revision);
    setChecked(undefined);
    save.reset();
    preview.reset();
  };
  const change = (value: Configuration) => {
    form.setValue("configuration", value);
    setChecked(undefined);
    if (preview.isError) preview.reset();
    if (save.isError) save.reset();
  };
  const previewConfiguration = form.handleSubmit(({ configuration }) =>
    preview.mutate(configuration, { onSuccess: setChecked }),
  );
  const saveConfiguration = (apply: boolean) => form.handleSubmit(({ configuration }) =>
    save.mutate({ apply, revision, configuration }, {
      onSuccess: (result) => {
        if (result.settings) {
          setRevision(result.settings.revision);
          form.reset({ configuration: result.settings.draft ?? result.settings.applied ?? EMPTY });
        }
        client.setQueryData(key, result);
        setChecked(undefined);
      },
    }),
  );

  const personal = settings.sources.find((source) => source.id === "personal_records");
  const sheets = settings.sources.find((source) => source.id === "google_sheets");
  const sheetConnections = settings.sheets_connections ?? [];
  const markForActivation = () => {
    prepare.mutate({ revision, skillSelection }, {
      onSuccess: (result) => {
        if (result.settings) setRevision(result.settings.revision);
        client.setQueryData(key, result);
        onPrepared();
      },
    });
  };
  const busy = save.isPending || preview.isPending || prepare.isPending;
  const ownerOnly = configuration.audience?.kind === "owner_only";
  const error = save.error ?? preview.error ?? prepare.error;
  const guests = settings.verified_guests;

  function addRule(source: Source) {
    const own = source.id === "personal_records";
    const connection = sheetConnections[0];
    const action = connection?.actions[0];
    if (!own && !action) return;
    const operations = Object.fromEntries(
      (own ? source.actions : action ? [action] : []).map((action) =>
        [action, defaultFields(source, action)]),
    );
    change({
      ...configuration,
      rules: [...configuration.rules, {
        source_id: source.id,
        collection_id: source.collection_id,
        audience: { kind: "all_guests" },
        scope: own ? { kind: "own_records" } : { kind: "resources", record_ids: [(connection?.id ?? "") + "//"] },
        delivery: own ? "private" : "shared",
        operations,
      }],
    });
  }

  return (
    <div className="space-y-4">
      <header className="space-y-1">
        <h2 className="type-card-title">{translateUi("Guest experience")}</h2>
        {staging && <p className="type-secondary text-muted-foreground">{translateUi("Prepare and preview a draft before enrollment. Applying a reviewed revision takes effect when this bot enters the rollout.")}</p>}
        {staging && settings.verified_guests.length === 0 && (
          <FieldHelp>{translateUi("Named guests are unavailable until this bot has verified channel observations.")}</FieldHelp>
        )}
        {staging && settings.unmapped_operations.length > 0 && (
          <details className="type-secondary">
            <summary>{translateUi("Existing guest operations without a replacement")}</summary>
            <ul className="list-disc space-y-1 pl-4">
              {settings.unmapped_operations.map((operation) =>
                <li key={`${operation.service}:${operation.method}:${operation.path}`}>
                  {operation.service} · <code>{operation.method} {operation.path}</code>
                </li>)}
            </ul>
          </details>
        )}
        {staging && settings.prepared_draft_revision != null && <p role="status" className="type-secondary text-muted-foreground">
          {translateUi("Draft revision {{revision}} is marked for activation.", { revision: settings.prepared_draft_revision })}
        </p>}
        <p className="type-secondary text-muted-foreground">
          {settings.draft
            ? translateUi("Draft saved. Continue here after connecting your bot or verifying guests.")
            : settings.applied
              ? translateUi("Policy revision {{revision}} applies {{count}} guest data rules.", { revision: settings.policy_revision, count: settings.effective_rule_count })
              : translateUi("Choose who can use this bot and what information it can share. Guests start with zero data grants.")}
        </p>
        {settings.draft && <p className="type-secondary text-muted-foreground">
          {translateUi("The current policy applies {{count}} guest data rules.", { count: settings.effective_rule_count })}
        </p>}
      </header>
      <Form {...form}>
      <fieldset disabled={busy || remoteChange} className="space-y-4">
        <div className="space-y-2">
          <Label htmlFor={id}>{translateUi("Who can talk to this bot?")}</Label>
          <NativeSelect id={id} value={configuration.audience?.kind ?? ""} onChange={(event) => {
            const kind = event.target.value;
            switch (kind) {
              case "owner_only":
                change({ audience: { kind }, rules: [], declarations: [] });
                break;
              case "channel_policy":
                change({ ...configuration, audience: { kind } });
                break;
              case "selected":
                change({ ...configuration, audience: { kind, guest_ids: [] } });
                break;
            }
          }}>
            <option value="" disabled>{translateUi("Choose an audience")}</option>
            <option value="owner_only">{translateUi("Owner only")}</option>
            <option value="selected" disabled={guests.length === 0 && configuration.audience?.kind !== "selected"}>{translateUi("Selected verified guests")}</option>
            <option value="channel_policy">{translateUi("Guests allowed by the channel settings")}</option>
          </NativeSelect>
          <FieldHelp>{translateUi("Existing channel admission and verified organization-member access still apply.")}</FieldHelp>
          {configuration.audience?.kind === "selected" && (
            <GuestChoices guests={guests} selected={configuration.audience.guest_ids} onChange={(ids) =>
              change({ ...configuration, audience: { kind: "selected", guest_ids: ids } })} />
          )}
        </div>
        {!ownerOnly && (
          <>
            <div className="flex flex-wrap gap-2">
              {personal && <Button variant="outline" onClick={() => addRule(personal)}>{translateUi("Manage their own data")}</Button>}
              {sheets && sheetConnections.length > 0 && <Button variant="outline" onClick={() => addRule(sheets)}>{translateUi("Share Google Sheets data")}</Button>}
            </div>
            <FieldHelp>
              {translateUi("Personal records use verified chat identities and private conversations. Connected services need explicit resource grants.")}
            </FieldHelp>
            {configuration.rules.map((rule, index) => (
              <RuleEditor key={index} rule={rule} index={index} settings={settings}
                onChange={(next) => change({ ...configuration, rules: configuration.rules.map((item, position) => position === index ? next : item) })}
                onRemove={() => change({ ...configuration, rules: configuration.rules.filter((_, position) => position !== index) })} />
            ))}
            {skills.data && skills.data.available.length > 0 && (
              <div className="space-y-2">
                <h3 className="type-card-title">{translateUi("Apply declarations from pinned skills")}</h3>
                <FieldHelp>{translateUi("Preview checks the structured declaration in the pinned package. Applying it changes guest access.")}</FieldHelp>
                {skills.data.available.map((pin) => {
                  const active = configuration.declarations.some((item) =>
                    item.skill_id === pin.skill_id && item.skill_revision === skills.data.skills_revision);
                  return (
                    <Label className="flex items-center gap-2" key={pin.skill_id}>
                      <Checkbox checked={active} onCheckedChange={(enabled) => {
                        const declarations = configuration.declarations.filter((item) => item.skill_id !== pin.skill_id);
                        if (enabled) declarations.push({ version: 1, skill_id: pin.skill_id, skill_revision: skills.data.skills_revision });
                        change({ ...configuration, declarations });
                      }} />
                      {pin.skill_id}
                    </Label>
                  );
                })}
              </div>
            )}
            {configuration.declarations.length > 0 && (
              <Button variant="outline" onClick={() => change({ ...configuration, declarations: [] })}>
                {translateUi("Remove skill declarations from this policy")}
              </Button>
            )}
          </>
        )}
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" disabled={!form.formState.isDirty} onClick={saveConfiguration(false)}>{translateUi("Save draft")}</Button>
          <Button variant="outline" disabled={!configuration.audience} onClick={previewConfiguration}>{translateUi("Preview two guests")}</Button>
          {staging
            ? <Button variant="primary" disabled={!checked || !settings.draft || form.formState.isDirty} onClick={markForActivation}>
                {translateUi("Mark draft for activation")}
              </Button>
            : <Button variant="primary" disabled={!checked} onClick={saveConfiguration(true)}>{translateUi("Apply guest setup")}</Button>}
        </div>
      </fieldset>
      <FormSubmitErrors />
      </Form>
      {remoteChange && (
        <div role="alert" className="space-y-2">
          <p className="type-secondary">{translateUi("Someone saved a newer guest setup. Reload it to continue editing.")}</p>
          <Button variant="outline" onClick={reload}>{translateUi("Reload guest settings")}</Button>
        </div>
      )}
      {checked && <PreviewReport report={checked} />}
      {error && (
        <div role="alert" className="space-y-2 type-secondary text-destructive">
          <p>{error instanceof ApiError ? error.message : translateUi("Guest settings could not be saved or checked.")}</p>
          {error instanceof ApiError && error.status === 409 && (
            <Button variant="outline" onClick={() => void client.invalidateQueries({ queryKey: key })}>
              {translateUi("Reload guest settings")}
            </Button>
          )}
        </div>
      )}
      <GuestSkillExposure botId={botId} onStage={staging ? setSkillSelection : undefined} />
    </div>
  );
}

function RuleEditor({ rule, index, settings, onChange, onRemove }: {
  readonly rule: Rule;
  readonly index: number;
  readonly settings: Settings;
  readonly onChange: (rule: Rule) => void;
  readonly onRemove: () => void;
}) {
  const id = useId();
  const source = settings.sources.find((value) => value.id === rule.source_id);
  const own = rule.scope.kind === "own_records";
  const rooms = settings.verified_rooms ?? [];
  const connections = settings.sheets_connections ?? [];
  const resource = rule.scope.kind === "resources" ? rule.scope.record_ids[0] ?? "" : "";
  const [connection = "", workbook = "", location = ""] = resource.split("/", 3);
  const matchingConnection = connections.find((item) => item.id === connection);
  const action = rule.operations.read ? "read" : "create";
  const updateResource = (nextConnection: string, nextWorkbook: string, nextLocation: string) =>
    onChange({ ...rule, scope: { kind: "resources", record_ids: [[nextConnection, nextWorkbook, nextLocation].join("/")] } });
  const actionOptions = [...new Set([...(matchingConnection?.actions ?? []), action])];
  const sharedOnly = !own && action === "create";
  const verifiedAudience = rule.audience.kind === "verified_audience" ? rule.audience : undefined;
  const roomId = verifiedAudience?.room_id ?? "";

  return (
    <fieldset className="space-y-3 rounded-lg border border-border p-3">
      <Label asChild><legend>{own ? translateUi("Personal records") : translateUi("Google Sheets")}</legend></Label>
      <div className="space-y-2">
        <Label htmlFor={id + "-audience"}>{translateUi("Who can use this rule?")}</Label>
        <NativeSelect id={id + "-audience"} value={rule.audience.kind} onChange={(event) => {
          switch (event.target.value) {
            case "all_guests":
              onChange({ ...rule, audience: { kind: "all_guests" }, delivery: own ? "private" : "shared" });
              break;
            case "guest":
              onChange({ ...rule, audience: { kind: "guest", guest_id: "" }, delivery: "private" });
              break;
            case "verified_audience":
              onChange({ ...rule, audience: { kind: "verified_audience", audience_id: "", guest_ids: [] }, delivery: own ? "private" : "shared" });
              break;
          }
        }}>
          <option value="all_guests">{translateUi("All admitted guests")}</option>
          {!sharedOnly && <option value="guest" disabled={settings.verified_guests.length === 0}>{translateUi("One verified guest")}</option>}
          <option value="verified_audience" disabled={settings.verified_guests.length === 0}>{translateUi("Named audience")}</option>
        </NativeSelect>
        {!own && !sharedOnly && rule.audience.kind !== "guest" && (
          <>
            <Label htmlFor={id + "-delivery"}>{translateUi("Where can the bot answer?")}</Label>
            <NativeSelect id={id + "-delivery"} value={rule.delivery} onChange={(event) => {
              const delivery = event.target.value === "private" ? "private" : "shared";
              const audience = verifiedAudience && delivery === "private"
                ? { ...verifiedAudience, room_id: undefined }
                : rule.audience;
              onChange({ ...rule, audience, delivery });
            }}>
              <option value="private">{translateUi("Private replies")}</option>
              <option value="shared">{translateUi("Shared replies")}</option>
            </NativeSelect>
          </>
        )}
        {rule.audience.kind === "guest" && (
          <>
            <Label htmlFor={id + "-guest"}>{translateUi("Verified guest")}</Label>
            <NativeSelect id={id + "-guest"} value={rule.audience.guest_id} onChange={(event) =>
              onChange({ ...rule, audience: { kind: "guest", guest_id: event.target.value } })}>
              <option value="">{translateUi("Select a verified guest")}</option>
              {[...new Set([...settings.verified_guests.map((guest) => guest.id), rule.audience.guest_id].filter(Boolean))].map((guest) =>
                <option key={guest} value={guest}>{guestLabel(guest, settings.verified_guests.find((candidate) => candidate.id === guest))}</option>)}
            </NativeSelect>
          </>
        )}
        {verifiedAudience && (
          <>
            <Label htmlFor={id + "-audience-name"}>{translateUi("Audience name")}</Label>
            <Input id={id + "-audience-name"} value={verifiedAudience.audience_id} onChange={(event) =>
              onChange({ ...rule, audience: { ...verifiedAudience, audience_id: event.target.value } })} />
            <GuestChoices guests={settings.verified_guests} selected={verifiedAudience.guest_ids} onChange={(guest_ids) =>
              onChange({ ...rule, audience: { ...verifiedAudience, guest_ids } })} />
            {!own && rule.delivery === "shared" && (
              <>
                <Label htmlFor={id + "-room"}>{translateUi("Verified room")}</Label>
                <NativeSelect id={id + "-room"} value={roomId} onChange={(event) =>
                  onChange({ ...rule, audience: { ...verifiedAudience, room_id: event.target.value || undefined } })}>
                  <option value="">{translateUi("Select a verified room")}</option>
                  {rooms.map((room) => <option key={room.id} value={room.id}>
                    {translateUi("{{platform}} room {{id}} · Seen {{date}}", {
                      platform: channelPlatformName(room.platform), id: room.id.slice(-12), date: formatDateTime(room.last_seen_at),
                    })}
                  </option>)}
                </NativeSelect>
                <FieldHelp>{translateUi("Everyone in the selected room can see the bot’s answer.")}</FieldHelp>
              </>
            )}
          </>
        )}
      </div>
      {own ? (
        source?.actions.map((operation) => {
          const fields = rule.operations[operation];
          const direction = operation === "read" ? "read" : "write";
          return <div key={operation} className="space-y-2">
            <Label className="flex items-center gap-2">
              <Checkbox checked={Boolean(fields)} onCheckedChange={(checked) => {
                const operations = { ...rule.operations };
                if (checked) operations[operation] = defaultFields(source, operation);
                else delete operations[operation];
                onChange({ ...rule, operations });
              }} />
              {actionLabel(operation)}
            </Label>
            {fields && operation !== "delete" && (
              <div className="flex flex-wrap gap-3 pl-6">
                {source.fields.map((field) => (
                  <Label key={field} className="flex items-center gap-2">
                    <Checkbox checked={fields[direction].includes(field)} onCheckedChange={(checked) =>
                      onChange({
                        ...rule,
                        operations: { ...rule.operations, [operation]: {
                          ...fields,
                          [direction]: checked ? [...fields[direction], field] : fields[direction].filter((value) => value !== field),
                        } },
                      })} />
                    {field === "title" ? translateUi("Title") : translateUi("Body")}
                    <span className="sr-only">{" "}{actionLabel(operation)}</span>
                  </Label>
                ))}
              </div>
            )}
          </div>;
        })
      ) : (
        <div className="grid gap-2">
          <Label htmlFor={id + "-connection"}>{translateUi("Google Sheets connection")}</Label>
          <NativeSelect id={id + "-connection"} value={connection} onChange={(event) => {
            const id = event.target.value;
            const nextAction = connections.find((item) => item.id === id)?.actions[0] ?? "read";
            onChange({
              ...rule,
              ...(nextAction === "create" ? sharedRuleAudience(rule) : {}),
              scope: { kind: "resources", record_ids: [[id, workbook, ""].join("/")] },
              operations: { [nextAction]: { read: [], write: [] } },
            });
          }}>
            <option value="">{translateUi("Select a connection")}</option>
            {connections.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}
            {connection && !matchingConnection && <option value={connection} disabled>{translateUi("Connection unavailable")}</option>}
          </NativeSelect>
          <Label htmlFor={id + "-action"}>{translateUi("What can guests do?")}</Label>
          <NativeSelect id={id + "-action"} value={action} onChange={(event) => {
            const selected = event.target.value as Action;
            onChange({
              ...rule,
              ...(selected === "create" ? sharedRuleAudience(rule) : {}),
              scope: { kind: "resources", record_ids: [[connection, workbook, ""].join("/")] },
              operations: { [selected]: { read: [], write: [] } },
            });
          }}>
            {actionOptions.map((option) => <option key={option} value={option}>
              {option === "read" ? translateUi("Read an exact cell range") : translateUi("Append one row to a tab")}
            </option>)}
          </NativeSelect>
          <Label htmlFor={id + "-workbook"}>{translateUi("Spreadsheet ID")}</Label>
          <Input id={id + "-workbook"} value={workbook ?? ""} onChange={(event) =>
            updateResource(connection, event.target.value, location)} />
          <FieldHelp>{translateUi("Copy the ID between /d/ and /edit in the spreadsheet URL.")}</FieldHelp>
          <Label htmlFor={id + "-resource"}>{action === "read" ? translateUi("Exact A1 range") : translateUi("Exact tab name")}</Label>
          <Input id={id + "-resource"} value={location} placeholder={action === "read" ? EXAMPLE_READ_RANGE : EXAMPLE_APPEND_TAB}
            onChange={(event) => updateResource(connection, workbook, event.target.value)} />
          {action === "create" && <FieldHelp>{translateUi("Appending is available in shared channel conversations. Google chooses where the new row goes within the named tab.")}</FieldHelp>}
        </div>
      )}
      <Button variant="ghost" size="sm" onClick={onRemove} aria-label={translateUi("Remove data rule {{number}}", { number: index + 1 })}>
        {translateUi("Remove rule")}
      </Button>
    </fieldset>
  );
}

function sharedRuleAudience(rule: Rule): Pick<Rule, "audience" | "delivery"> {
  return {
    audience: rule.audience.kind === "guest"
      ? { kind: "verified_audience", audience_id: "", guest_ids: [rule.audience.guest_id] }
      : rule.audience,
    delivery: "shared",
  };
}

function PreviewReport({ report }: { readonly report: Preview }) {
  const decision = (allowed: boolean | null) => allowed === null ? "—" : allowed ? translateUi("Allowed") : translateUi("Denied");
  return (
    <div aria-live="polite" className="space-y-2 rounded-lg border border-border p-3">
      <h3 className="type-card-title">{translateUi("Two-guest policy preview")}</h3>
      <p className="type-secondary text-muted-foreground">{translateUi("The server evaluated this configuration with sample records. Live data, delivery, and independent approval checks require a real channel test.")}</p>
      <p className="type-body">
        {translateUi("Guest A admission: {{a}}. Guest B admission: {{b}}.", { a: decision(report.guests_admitted[0]), b: decision(report.guests_admitted[1]) })}
      </p>
      {report.rule_count === 0 && <p className="type-body">{translateUi("No guest data grants.")}</p>}
      {report.checks.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-left type-body">
            <thead>
              <tr>
                <th scope="col">{translateUi("Action")}</th>
                <th scope="col">{translateUi("Guest A in private")}</th>
                <th scope="col">{translateUi("Guest B in private")}</th>
                <th scope="col">{translateUi("Guest B on A’s record")}</th>
                <th scope="col">{translateUi("Guest A in group")}</th>
              </tr>
            </thead>
            <tbody>
              {report.checks.map((check, index) => (
                <tr key={index}>
                  <th scope="row" className="py-1 pr-2">
                    {check.source_id === "personal_records" ? translateUi("Personal records") : translateUi("Google Sheets")}
                    {" · "}{actionLabel(check.action)}
                    {check.resource && <span className="block type-secondary text-muted-foreground">{check.resource}</span>}
                    {check.read_fields.length > 0 && <span className="block type-secondary text-muted-foreground">
                      {translateUi("Readable fields: {{fields}}", { fields: check.read_fields.join(", ") })}
                    </span>}
                    {check.write_fields.length > 0 && <span className="block type-secondary text-muted-foreground">
                      {translateUi("Writable fields: {{fields}}", { fields: check.write_fields.join(", ") })}
                    </span>}
                  </th>
                  <td>{decision(check.guest_a)}</td>
                  <td>{decision(check.guest_b)}</td>
                  <td>{decision(check.guest_b_on_a)}</td>
                  <td>{decision(check.guest_a_in_group)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
