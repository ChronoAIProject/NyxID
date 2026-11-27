import { StoredApprovalPolicyDialog } from "./stored-approval-policy-dialog";
import { ApprovalTelegramSetup } from "./approval-telegram-setup";
import { useEffect, useId, useMemo, useState, type KeyboardEvent } from "react";
import { ChevronRight, Search, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { ApiError } from "@/lib/api-client";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { Input } from "@/components/ui/input";
import { FieldHelp, Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
} from "@/components/ui/segmented-control";
import { SectionInfo } from "@/components/ui/section-info";
import { Skeleton } from "@/components/ui/skeleton";
import { StepDots } from "@/components/ui/step-dots";
import { Switch } from "@/components/ui/switch";
import { ServiceIcon } from "@/components/service-icon";
import {
  useApprovalAuthoringCapabilities,
  useDeleteServiceApprovalConfig,
  useApprovalGrants,
  useNotificationSettings,
  useRevokeGrant,
  useServiceApprovalConfigs,
  useSetServiceApprovalConfig,
  useUpdateNotificationSettings,
} from "@/hooks/use-approvals";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useEndpoints } from "@/hooks/use-endpoints";
import { useUserServices } from "@/hooks/use-user-services";
import {
  MAX_PROTECTION_RULES,
  actionVerb,
  buildPolicy,
  effectiveSetting,
  isEmptyPolicy,
  isWizardPolicy,
  newProtection,
  parsePolicy,
  sameScope,
  withActionSetting,
  withBase,
  withKeys,
  withProtection,
  withoutProtection,
  type ActionChoice,
  type ActionSetting,
  type BaseChoice,
  type ProtectionAction,
  type ProtectionDraft,
} from "@/lib/approval-protection";
import type { ApiKey } from "@/types/api";
import type {
  GlobalApprovalMode,
  NotificationSettings,
  ServiceApprovalConfigItem,
} from "@/types/approvals";

/** A service the owner can protect, with the id the policy API expects. */
interface ProtectableService {
  readonly id: string;
  readonly label: string;
  readonly detail: string;
  readonly slug: string;
  readonly catalogServiceId: string | null;
  readonly config: ServiceApprovalConfigItem | null;
}

/** One protection: a service, and the agent keys it is limited to (none means all). */
interface ProtectionScope {
  readonly service: ProtectableService;
  readonly keyIds: readonly string[];
}

function keyNames(
  keyIds: readonly string[],
  keys: readonly ApiKey[],
): readonly string[] {
  return keyIds.map(
    (id) =>
      keys.find((key) => key.id === id)?.name ?? translateUi("Deleted key"),
  );
}

interface WhereChoice {
  readonly push: boolean;
  readonly telegram: boolean;
}

const BASE_OPTIONS: readonly {
  readonly value: BaseChoice;
  readonly title: string;
  readonly detail: string;
}[] = [
  {
    value: "ask_changes",
    title: "Changes only",
    detail:
      "Reads run without asking. Anything that changes or deletes needs your approval.",
  },
  {
    value: "ask_all",
    title: "Every request",
    detail:
      "Every request to this service needs your approval, including reads.",
  },
  {
    value: "allow_all",
    title: "Only actions I pick",
    detail:
      "Everything runs without asking, except the actions you set under Customise actions.",
  },
];

const STEPS = ["service", "keys", "actions", "where", "review"] as const;
const STEP_NAMES: Record<(typeof STEPS)[number], string> = {
  service: "Service",
  keys: "Applies to",
  actions: "Actions",
  where: "Where you are asked",
  review: "Review",
};

/** Whole days only: the server keeps a remembered approval for 1 to 365 days. */
const DAY_PRESETS: readonly number[] = [1, 7, 30, 90];

const BASE_LABELS: Record<BaseChoice, string> = {
  ask_changes: "Changes only",
  ask_all: "Every request",
  allow_all: "Only picked actions",
};

const CHOICE_LABELS: Record<ActionChoice, string> = {
  allow: "Allow",
  ask: "Ask me",
  block: "Block",
};

/** One value per action: the choice and, for Ask, how often. */
type ActionOption = "allow" | "ask_every" | "ask_once" | "block";

const ACTION_OPTIONS: readonly {
  readonly value: ActionOption;
  /** Short label on the button; `hint` carries the full meaning. */
  readonly label: string;
  readonly hint: string;
  readonly setting: ActionSetting;
}[] = [
  {
    value: "allow",
    label: "Allow",
    hint: "Runs without asking",
    setting: { choice: "allow", everyTime: true },
  },
  {
    value: "ask_every",
    label: "Ask",
    hint: "Asks you every time",
    setting: { choice: "ask", everyTime: true },
  },
  {
    value: "ask_once",
    label: "Once",
    hint: "Asks once, then remembers your approval",
    setting: { choice: "ask", everyTime: false },
  },
  {
    value: "block",
    label: "Block",
    hint: "Always refused",
    setting: { choice: "block", everyTime: true },
  },
];

function actionOption(setting: ActionSetting): ActionOption {
  if (setting.choice === "ask")
    return setting.everyTime ? "ask_every" : "ask_once";
  return setting.choice;
}

function errorText(error: unknown, fallback: string): string {
  return translateErrorMessage(
    error instanceof ApiError ? error.message : fallback,
  );
}

/** Catalog operation names are identifiers (`create_issue`); show them as words. */
function actionLabel(name: string): string {
  const words = name.replace(/[_-]+/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : name;
}

function timeoutLabel(seconds: number): string {
  return seconds >= 120 && seconds % 60 === 0
    ? translateUi("{{count}} minutes", { count: seconds / 60 })
    : translateUi("{{count}} seconds", { count: seconds });
}

function whereSummary(where: WhereChoice): string {
  const places = [translateUi("This Approvals page")];
  if (where.push) places.push(translateUi("Mobile app"));
  if (where.telegram) places.push(translateUi("Telegram"));
  return places.join(", ");
}

/** The filled or empty circle that marks a card as one choice among several. */
function moveRadio(event: KeyboardEvent<HTMLElement>) {
  if (
    ![
      "ArrowLeft",
      "ArrowRight",
      "ArrowUp",
      "ArrowDown",
      "Home",
      "End",
    ].includes(event.key)
  )
    return;
  const options = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>(
      '[role="radio"]:not(:disabled)',
    ),
  );
  const current = options.indexOf(event.target as HTMLButtonElement);
  if (!options.length || current < 0) return;
  event.preventDefault();
  const next =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? options.length - 1
        : (current +
            (["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 1) +
            options.length) %
          options.length;
  options[next]?.focus();
  options[next]?.click();
}

function RadioDot({ checked }: { readonly checked: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex size-4 shrink-0 items-center justify-center rounded-full border",
        checked ? "border-primary" : "border-input",
      )}
    >
      {checked ? <span className="size-2 rounded-full bg-primary" /> : null}
    </span>
  );
}

function SearchField({
  label,
  value,
  onChange,
}: {
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  return (
    <div className="relative">
      <Search
        aria-hidden
        className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-text-tertiary"
      />
      <Input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={label}
        aria-label={label}
        className="pl-9"
      />
    </div>
  );
}

function ActionChoiceGroup({
  label,
  value,
  onChange,
  disabledOptions = [],
}: {
  readonly label: string;
  readonly value: ActionSetting;
  readonly onChange: (value: ActionSetting) => void;
  readonly disabledOptions?: readonly ActionOption[];
}) {
  const current = actionOption(value);
  return (
    <div
      role="radiogroup"
      onKeyDown={moveRadio}
      aria-label={label}
      className="inline-flex shrink-0 overflow-hidden rounded-md border border-hairline"
    >
      {ACTION_OPTIONS.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={current === option.value}
          tabIndex={current === option.value ? 0 : -1}
          title={translateUi(option.hint)}
          disabled={disabledOptions.includes(option.value)}
          onClick={() => onChange(option.setting)}
          className={cn(
            "min-h-6 border-l border-hairline px-2.5 py-1 type-secondary-emphasis first:border-l-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-40",
            current === option.value
              ? "bg-overlay-strong text-foreground"
              : "text-text-tertiary hover:bg-overlay hover:text-foreground",
          )}
        >
          {translateUi(option.label)}
        </button>
      ))}
    </div>
  );
}

function WhereControls({
  settings,
  value,
  onChange,
  disabled,
}: {
  readonly settings: NotificationSettings;
  readonly value: WhereChoice;
  readonly onChange: (changes: Partial<WhereChoice>) => void;
  readonly disabled?: boolean;
}) {
  const hasPhone = settings.push_device_count > 0;
  const [waitingForPhone, setWaitingForPhone] = useState(false);
  useNotificationSettings({ enabled: waitingForPhone && !hasPhone, refetchInterval: 5000 });
  useEffect(() => {
    if (!waitingForPhone || hasPhone) return;
    const timer = window.setTimeout(() => setWaitingForPhone(false), 5 * 60_000);
    return () => window.clearTimeout(timer);
  }, [waitingForPhone, hasPhone]);
  return (
    <GroupedList>
      <GroupedRow
        title={translateUi("This Approvals page")}
        description={translateUi(
          "Always on, so a request can always be answered.",
        )}
        accessory={
          <Switch
            checked
            disabled
            aria-label={translateUi("This Approvals page")}
          />
        }
      />
      <GroupedRow
        title={translateUi("Mobile app")}
        description={
          hasPhone
            ? translateUi(
                "Push notification to your phone, then Approve or Reject in the app.",
              )
            : translateUi(
                "No phone is registered yet. Install the NyxID app and sign in to receive requests.",
              )
        }
        accessory={
          hasPhone ? (
            <Switch
              aria-label={translateUi("Mobile app")}
              checked={value.push}
              disabled={disabled}
              onCheckedChange={(push) => onChange({ push })}
            />
          ) : (
            <Button asChild variant="outline" size="sm">
              <a
                href="https://apps.apple.com/app/nyxid/id6759768423"
                onClick={() => setWaitingForPhone(true)}
                target="_blank"
                rel="noopener noreferrer"
              >
                {translateUi("Get the app")}
              </a>
            </Button>
          )
        }
      />
      {settings.telegram_connected ? (
        <GroupedRow
          title={translateUi("Telegram")}
          description={translateUi("Approve and Reject buttons in your linked Telegram chat.")}
          accessory={
            <Switch
              aria-label={translateUi("Telegram")}
              checked={value.telegram}
              disabled={disabled}
              onCheckedChange={(telegram) => onChange({ telegram })}
            />
          }
        />
      ) : <ApprovalTelegramSetup disabled={disabled} />}
    </GroupedList>
  );
}

function ProtectionSetupDialog({
  services,
  initial,
  settings,
  rulesReady,
  onClose,
  onEditStored,
}: {
  readonly onEditStored: (config: ServiceApprovalConfigItem) => void;
  /** Key-limited protections need every server to support them. */
  readonly rulesReady: boolean;
  readonly services: readonly ProtectableService[];
  /** The protection to edit; null starts a new one. */
  readonly initial: ProtectionScope | null;
  readonly settings: NotificationSettings;
  readonly onClose: () => void;
}) {
  const baseId = useId();
  const oftenId = useId();
  // Editing opens on Actions; a new protection starts by picking the service.
  const editing = initial !== null;
  const [step, setStep] = useState(editing ? 3 : 1);
  const at = STEPS[step - 1] ?? "service";
  const apiKeys = useApiKeys();
  const keys = (apiKeys.data ?? []).filter((key) => key.is_active);
  const [limitChosen, setLimitChosen] = useState<boolean | null>(null);
  // Each protection keeps its own number of days; a new one starts from the account setting.
  const initialDays =
    initial?.service.config?.grant_expiry_days ?? settings.grant_expiry_days;
  const [rememberDays, setRememberDays] = useState(initialDays);
  const [customDays, setCustomDays] = useState(
    !DAY_PRESETS.includes(initialDays),
  );
  const [service, setService] = useState<ProtectableService | null>(
    initial?.service ?? null,
  );
  const [confirmingOverride, setConfirmingOverride] = useState(false);
  const [serviceQuery, setServiceQuery] = useState("");
  const [actionQuery, setActionQuery] = useState("");
  const [customising, setCustomising] = useState(false);
  const [edited, setEdited] = useState<ProtectionDraft | null>(null);
  const [scopeDrafts, setScopeDrafts] = useState<{
    all?: ProtectionDraft;
    specific?: ProtectionDraft;
  }>({});
  // Linking a channel can enable delivery while this draft is open.
  const [deliveryEdits, setDeliveryEdits] = useState<Partial<WhereChoice>>({});
  const where = {
    push: deliveryEdits.push ?? settings.push_enabled,
    telegram: deliveryEdits.telegram ?? settings.telegram_enabled,
  };
  const endpoints = useEndpoints(service?.catalogServiceId ?? "");
  const save = useSetServiceApprovalConfig();
  const remove = useDeleteServiceApprovalConfig();
  const updateSettings = useUpdateNotificationSettings();
  const busy = save.isPending || remove.isPending || updateSettings.isPending;

  const actions: readonly ProtectionAction[] = useMemo(
    () =>
      (endpoints.data ?? [])
        .filter((endpoint) => endpoint.is_active)
        .map((endpoint) => ({
          id: endpoint.id,
          name: actionLabel(endpoint.name),
          method: endpoint.method,
          path: endpoint.path,
        })),
    [endpoints.data],
  );
  const policy = useMemo(
    () => parsePolicy(service?.config ?? null, actions),
    [service, actions],
  );
  // "Specific keys" stays selected while the list is still empty.
  const limited = limitChosen ?? (initial?.keyIds.length ?? 0) > 0;
  // Editing loads that protection. A new All keys protection loads the existing
  // one for the service, so there is never a second copy to reconcile.
  const stored = useMemo((): ProtectionDraft => {
    if (initial) {
      return (
        (initial.keyIds.length
          ? policy.keyed.find((entry) =>
              sameScope(entry.keyIds, initial.keyIds),
            )
          : policy.all) ?? newProtection(initial.keyIds)
      );
    }
    return limited ? newProtection() : (policy.all ?? newProtection());
  }, [initial, limited, policy]);
  const draft = edited ?? stored;
  const others = initial ? withoutProtection(policy, initial.keyIds) : policy;
  const nextPolicy = withProtection(others, draft);
  const payload = buildPolicy(nextPolicy, actions);
  const keyRulesReady =
    rulesReady ||
    (policy.keyed.length === 0 &&
      !payload.rules.some((rule) => (rule.api_key_ids?.length ?? 0) > 0));
  const tooManyRules = payload.rules.length > MAX_PROTECTION_RULES;
  // Keys that already have their own protection for this service.
  const takenKeyIds = others.keyed.flatMap((entry) => entry.keyIds);
  // A key's own rules win over the All keys rules; say so before saving both.
  const overridesGeneral =
    nextPolicy.all !== null && nextPolicy.keyed.length > 0;
  const replacingGeneral = !editing && !limited && policy.all !== null;
  // Shown always, so the setting is never a surprise; locked when it cannot apply.
  const howOftenLocked = draft.keyIds.length > 0 || draft.base === "allow_all";
  // Before every server supports per-protection days, a remembered approval
  // uses the account setting and the request carries no day override.
  const days = rulesReady
    ? Math.min(365, Math.max(1, Math.round(rememberDays) || 1))
    : settings.grant_expiry_days;
  const dayPresets = rulesReady ? DAY_PRESETS : [settings.grant_expiry_days];
  const oftenValue =
    !draft.remember || howOftenLocked
      ? "every_time"
      : customDays && rulesReady
        ? "custom"
        : String(days);
  const actionsLoading =
    Boolean(service?.catalogServiceId) && endpoints.isLoading;
  const safeEdit =
    !service?.config ||
    (!actionsLoading &&
      !endpoints.isError &&
      isWizardPolicy(service.config, actions));

  const storedEditControl =
    service?.config && !safeEdit && !actionsLoading ? (
      <div className="space-y-2">
        <p className="type-secondary text-text-tertiary">
          {translateUi(
            "This saved policy uses ordered rules. Edit the stored policy to preserve its meaning.",
          )}
        </p>
        <Button variant="outline" onClick={() => onEditStored(service.config!)}>
          {translateUi("Edit stored policy")}
        </Button>
      </div>
    ) : null;
  const visibleServices = services.filter((entry) =>
    `${entry.label} ${entry.detail}`
      .toLowerCase()
      .includes(serviceQuery.trim().toLowerCase()),
  );
  const visibleActions = actions.filter((action) =>
    `${action.name} ${action.method} ${action.path}`
      .toLowerCase()
      .includes(actionQuery.trim().toLowerCase()),
  );
  const picked = actions
    .filter((action) => draft.overrides[action.id])
    .map((action) => ({ action, setting: draft.overrides[action.id]! }));

  function chooseService(entry: ProtectableService) {
    setService(entry);
    setEdited(null);
    setScopeDrafts({});
    setActionQuery("");
    setCustomising(false);
    setLimitChosen(null);
    setConfirmingOverride(false);
    const entryDays =
      entry.config?.grant_expiry_days ?? settings.grant_expiry_days;
    setRememberDays(entryDays);
    setCustomDays(!DAY_PRESETS.includes(entryDays));
  }

  async function turnOn() {
    if (!service || !safeEdit || !keyRulesReady) return;
    setConfirmingOverride(false);
    try {
      await save.mutateAsync({
        serviceId: service.id,
        expectedRevision: service.config?.revision ?? "",
        approvalRequired: payload.approvalRequired,
        approvalMode: payload.approvalMode,
        rules: payload.rules,
        defaultEffect: payload.defaultEffect,
        ...(rulesReady &&
        draft.keyIds.length === 0 &&
        draft.remember &&
        days !==
          (service.config?.grant_expiry_days ?? settings.grant_expiry_days)
          ? { grantExpiryDays: days }
          : {}),
      });
      if (
        where.push !== settings.push_enabled ||
        where.telegram !== settings.telegram_enabled
      ) {
        await updateSettings.mutateAsync({
          ...(deliveryEdits.push !== undefined && { push_enabled: where.push }),
          ...(deliveryEdits.telegram !== undefined && { telegram_enabled: where.telegram }),
        });
      }
      toast.success(
        editing
          ? translateUi("Protection updated for {{service}}", {
              service: service.label,
            })
          : translateUi("Protection is on for {{service}}", {
              service: service.label,
            }),
      );
      onClose();
    } catch (error) {
      toast.error(
        translateErrorMessage(
          errorText(error, "Failed to save the protection"),
        ),
      );
    }
  }

  /** Removes only this protection; the service's other protections stay. */
  async function removeProtection() {
    if (!service || !initial) return;
    const rest = withoutProtection(policy, initial.keyIds);
    try {
      if (isEmptyPolicy(rest)) {
        await remove.mutateAsync({ serviceId: service.id });
      } else {
        const remaining = buildPolicy(rest, actions);
        await save.mutateAsync({
          serviceId: service.id,
          approvalRequired: remaining.approvalRequired,
          approvalMode: remaining.approvalMode,
          rules: remaining.rules,
          defaultEffect: remaining.defaultEffect,
        });
      }
      toast.success(
        translateUi("Protection removed from {{service}}", {
          service: service.label,
        }),
      );
      onClose();
    } catch (error) {
      toast.error(
        translateErrorMessage(
          errorText(error, "Failed to remove the protection"),
        ),
      );
    }
  }

  const scopeLabel = draft.keyIds.length
    ? keyNames(draft.keyIds, keys).join(", ")
    : translateUi("All keys");

  const titles = [
    translateUi("Which service do you want to protect?"),
    translateUi("Who does this apply to?"),
    translateUi("Which {{service}} actions need your approval?", {
      service: service?.label ?? "",
    }),
    translateUi("Where should we ask you?"),
    editing
      ? translateUi("Review and save")
      : translateUi("Review and turn on"),
  ];

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent scrollMode="body" className="md:max-w-xl">
        <DialogHeader>
          <StepDots
            label={translateUi("Setup steps")}
            steps={STEPS.map((name) => translateUi(STEP_NAMES[name]))}
            index={step - 1}
            canSelect={(position) =>
              position < step - 1 && !(editing && position === 0)
            }
            onSelect={(position) => setStep(position + 1)}
            showCaption
            className="-mx-0.75"
          />
          <DialogTitle>
            {at === "actions" && customising
              ? translateUi("Customise {{service}} actions", {
                  service: service?.label ?? "",
                })
              : titles[step - 1]}
          </DialogTitle>
          {editing ? (
            <DialogDescription>
              {translateUi(
                "Editing the {{service}} protection for {{scope}}.",
                {
                  service: service?.label ?? "",
                  scope: scopeLabel,
                },
              )}
            </DialogDescription>
          ) : (
            <DialogDescription className="sr-only">
              {translateUi("Set up which actions need your approval.")}
            </DialogDescription>
          )}
        </DialogHeader>

        <DialogBody className="space-y-4 py-4">
          {storedEditControl}

          {at === "service" ? (
            <>
              <SearchField
                label={translateUi("Search your connected services")}
                value={serviceQuery}
                onChange={setServiceQuery}
              />
              {visibleServices.length ? (
                <div
                  role="radiogroup"
                  onKeyDown={moveRadio}
                  aria-label={titles[0]}
                  className="space-y-2"
                >
                  {visibleServices.map((entry) => (
                    <button
                      key={entry.id}
                      type="button"
                      role="radio"
                      aria-checked={service?.id === entry.id}
                      tabIndex={
                        service?.id === entry.id ||
                        (!service && entry === visibleServices[0])
                          ? 0
                          : -1
                      }
                      onClick={() => chooseService(entry)}
                      className={cn(
                        "flex w-full items-center gap-3 rounded-lg border px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                        service?.id === entry.id
                          ? "border-primary bg-overlay"
                          : "border-hairline hover:border-hairline-strong hover:bg-overlay",
                      )}
                    >
                      <RadioDot checked={service?.id === entry.id} />
                      <ServiceIcon slug={entry.slug} size="sm" />
                      <span className="min-w-0 flex-1">
                        <span className="block type-row-title text-foreground">
                          {entry.label}
                        </span>
                        <span className="block type-secondary text-text-tertiary">
                          {entry.detail}
                        </span>
                      </span>
                      {entry.config ? (
                        <Badge variant="secondary">
                          {translateUi("Protected")}
                        </Badge>
                      ) : null}
                    </button>
                  ))}
                </div>
              ) : (
                <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center type-secondary text-text-tertiary">
                  {services.length
                    ? translateUi("No connected service matches your search.")
                    : translateUi(
                        "Connect a service first, then come back to protect it.",
                      )}
                </p>
              )}
            </>
          ) : null}

          {at === "keys" ? (
            <>
              <p className="type-secondary text-text-tertiary">
                {translateUi("Choose one.")}
              </p>
              <div
                role="radiogroup"
                onKeyDown={moveRadio}
                aria-label={titles[1]}
                className="space-y-2"
              >
                {[
                  {
                    limit: false,
                    title: translateUi("All keys"),
                    detail: translateUi(
                      "Every agent key and app that uses this service follows these rules.",
                    ),
                  },
                  {
                    limit: true,
                    title: translateUi("Specific agent keys"),
                    detail: translateUi(
                      "Only the keys you pick follow these rules, and they ignore any All keys rules for this service.",
                    ),
                  },
                ].map((option) => {
                  const selected = limited === option.limit;
                  return (
                    <div
                      key={String(option.limit)}
                      className={cn(
                        "rounded-lg border",
                        selected
                          ? "border-primary bg-overlay"
                          : "border-hairline",
                      )}
                    >
                      <button
                        type="button"
                        role="radio"
                        aria-checked={selected}
                        tabIndex={
                          selected || (limitChosen === null && !option.limit)
                            ? 0
                            : -1
                        }
                        // An existing protection keeps its kind; only its keys can change.
                        disabled={
                          (editing && !selected) || (option.limit && !rulesReady)
                        }
                        onClick={() => {
                          if (selected) return;
                          setScopeDrafts((current) => ({
                            ...current,
                            [limited ? "specific" : "all"]: draft,
                          }));
                          setLimitChosen(option.limit);
                          setEdited(
                            scopeDrafts[option.limit ? "specific" : "all"] ??
                              withKeys(draft, [], actions),
                          );
                        }}
                        className={cn(
                          "flex w-full items-start gap-3 rounded-lg px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
                          !selected && "hover:bg-overlay",
                        )}
                      >
                        <span className="mt-0.5">
                          <RadioDot checked={selected} />
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block type-row-title text-foreground">
                            {option.title}
                          </span>
                          <span className="block type-secondary text-text-tertiary">
                            {option.detail}
                          </span>
                        </span>
                      </button>
                      {option.limit && selected ? (
                        <div className="space-y-2 border-t border-border/50 px-3 py-3 pl-10">
                          <p className="type-secondary-emphasis text-foreground">
                            {translateUi("Which keys?")}
                          </p>
                          {apiKeys.isLoading ? (
                            <Skeleton className="h-24 w-full" />
                          ) : apiKeys.isError ? (
                            <p
                              role="alert"
                              className="type-secondary text-destructive"
                            >
                              {translateUi(
                                "Could not load agent keys. Retry before choosing keys.",
                              )}
                            </p>
                          ) : keys.length ? (
                            <div className="space-y-2">
                              <ul className="divide-y divide-border/50 rounded-lg border border-border/50">
                                {keys.map((key) => {
                                  const checked = draft.keyIds.includes(key.id);
                                  const taken = takenKeyIds.includes(key.id);
                                  return (
                                    <li key={key.id}>
                                      <Label
                                        className={cn(
                                          "flex items-center gap-3 px-3 py-2",
                                          taken
                                            ? "cursor-not-allowed opacity-60"
                                            : "cursor-pointer",
                                        )}
                                      >
                                        <Checkbox
                                          checked={checked}
                                          disabled={
                                            !rulesReady ||
                                            taken ||
                                            (!checked &&
                                              draft.keyIds.length >= 20)
                                          }
                                          onCheckedChange={(next) =>
                                            setEdited(
                                              withKeys(
                                                draft,
                                                next === true
                                                  ? [...draft.keyIds, key.id]
                                                  : draft.keyIds.filter(
                                                      (id) => id !== key.id,
                                                    ),
                                                actions,
                                              ),
                                            )
                                          }
                                        />
                                        <span className="min-w-0 flex-1">
                                          <span className="block break-words text-foreground">
                                            {key.name}
                                          </span>
                                          <FieldHelp>
                                            {[
                                              key.platform,
                                              key.key_prefix,
                                              taken
                                                ? translateUi(
                                                    "already has its own protection for this service",
                                                  )
                                                : null,
                                            ]
                                              .filter(Boolean)
                                              .join(" · ")}
                                          </FieldHelp>
                                        </span>
                                      </Label>
                                    </li>
                                  );
                                })}
                              </ul>
                              <p className="type-secondary text-text-tertiary">
                                {translateUi(
                                  "Picked keys are asked every time. A remembered approval would also cover your other keys.",
                                )}
                              </p>
                            </div>
                          ) : (
                            <p className="rounded-lg border border-dashed border-border px-4 py-4 type-secondary text-text-tertiary">
                              {translateUi(
                                "You have no agent keys yet. Create one first, or apply this to all keys.",
                              )}
                            </p>
                          )}
                        </div>
                      ) : null}
                    </div>
                  );
                })}
              </div>

              {replacingGeneral ? (
                <p className="type-secondary text-text-tertiary">
                  {translateUi(
                    "This service already has an All keys protection. Continuing edits that one.",
                  )}
                </p>
              ) : null}
            </>
          ) : null}

          {at === "actions" && !customising ? (
            <>
              <div className="space-y-1.5">
                <div className="flex items-center gap-1 [&>button]:!mt-0">
                  <Label htmlFor={baseId}>
                    {translateUi("Require approval for")}
                  </Label>
                  <SectionInfo
                    title={translateUi("Require approval for")}
                    align="start"
                  >
                    {translateUi(
                      "Sets what happens to every request to this service. You can still set individual actions under Customise actions.",
                    )}
                  </SectionInfo>
                </div>
                <Select
                  value={draft.base}
                  disabled={actionsLoading || endpoints.isError || !safeEdit}
                  onValueChange={(value) =>
                    setEdited(
                      withBase(draft, { base: value as BaseChoice }, actions),
                    )
                  }
                >
                  <SelectTrigger id={baseId} className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {BASE_OPTIONS.map((option) => (
                      <SelectItem
                        key={option.value}
                        value={option.value}
                        description={translateUi(option.detail)}
                      >
                        {option.value === "ask_changes"
                          ? translateUi("{{title}} (recommended)", {
                              title: translateUi(option.title),
                            })
                          : translateUi(option.title)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="type-secondary text-text-tertiary">
                  {translateUi(
                    BASE_OPTIONS.find((option) => option.value === draft.base)!
                      .detail,
                  )}
                </p>
              </div>

              <div className="space-y-1.5">
                <Label htmlFor={oftenId}>{translateUi("Ask again")}</Label>
                <div className="flex flex-wrap items-center gap-2">
                  <Select
                    value={oftenValue}
                    disabled={howOftenLocked}
                    onValueChange={(value) => {
                      if (value === "every_time") {
                        setCustomDays(false);
                        setEdited(
                          withBase(draft, { remember: false }, actions),
                        );
                        return;
                      }
                      setCustomDays(value === "custom");
                      if (value !== "custom") setRememberDays(Number(value));
                      setEdited(withBase(draft, { remember: true }, actions));
                    }}
                  >
                    <SelectTrigger
                      id={oftenId}
                      className="min-w-0 flex-1 basis-48"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem
                        value="every_time"
                        description={translateUi(
                          "Each request needs its own approve or reject.",
                        )}
                      >
                        {translateUi("Every time")}
                      </SelectItem>
                      {dayPresets.map((preset) => (
                        <SelectItem key={preset} value={String(preset)}>
                          {preset === 1
                            ? translateUi("After 1 day")
                            : translateUi("After {{count}} days", {
                                count: preset,
                              })}
                        </SelectItem>
                      ))}
                      {rulesReady ? (
                        <SelectItem value="custom">
                          {translateUi("Custom number of days")}
                        </SelectItem>
                      ) : null}
                    </SelectContent>
                  </Select>
                  {oftenValue === "custom" ? (
                    <>
                      <Input
                        type="number"
                        inputMode="numeric"
                        min={1}
                        max={365}
                        step={1}
                        value={rememberDays}
                        aria-label={translateUi("Number of days")}
                        onChange={(event) =>
                          setRememberDays(event.target.valueAsNumber)
                        }
                        onBlur={() => setRememberDays(days)}
                        className="w-20"
                      />
                      <span className="type-secondary text-text-tertiary">
                        {translateUi("days (1 to 365)")}
                      </span>
                    </>
                  ) : null}
                </div>
                <p className="type-secondary text-text-tertiary">
                  {draft.keyIds.length
                    ? translateUi(
                        "Fixed to every time because this protection is limited to specific keys. A remembered approval would also cover your other keys.",
                      )
                    : draft.base === "allow_all"
                      ? translateUi(
                          "Nothing is asked by default here. Choose Ask or Once for individual actions under Customise actions.",
                        )
                      : draft.remember
                        ? days === 1
                          ? translateUi(
                              "After you approve an action once, it can repeat without asking for 1 day. Other protections keep their own number of days.",
                            )
                          : translateUi(
                              "After you approve an action once, it can repeat without asking for {{count}} days. Other protections keep their own number of days.",
                              { count: days },
                            )
                        : translateUi(
                            "Nothing is remembered between requests.",
                          )}
                </p>
              </div>

              {endpoints.isError && (
                <p role="alert" className="type-secondary text-destructive">
                  {translateUi(
                    "Could not load published actions. Retry before saving.",
                  )}
                </p>
              )}
              {actionsLoading ? (
                <Skeleton className="h-14 w-full" />
              ) : actions.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border px-4 py-4 type-secondary text-text-tertiary">
                  {translateUi(
                    "This service has no published action list, so the choice above applies to all of its requests.",
                  )}
                </p>
              ) : (
                <button
                  type="button"
                  onClick={() => setCustomising(true)}
                  className="flex w-full items-center gap-2 rounded-lg border border-hairline px-3 py-2 text-left hover:border-hairline-strong hover:bg-overlay focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <span className="min-w-0 flex-1">
                    <span className="block type-row-title text-foreground">
                      {translateUi("Customise actions")}
                    </span>
                    <span className="block type-secondary text-text-tertiary">
                      {picked.length
                        ? translateUi(
                            "{{count}} of {{total}} actions set individually",
                            {
                              count: picked.length,
                              total: actions.length,
                            },
                          )
                        : translateUi(
                            "Optional. Choose what happens for any of {{total}} actions.",
                            {
                              total: actions.length,
                            },
                          )}
                    </span>
                  </span>
                  <ChevronRight
                    aria-hidden
                    className="size-4 text-text-tertiary"
                  />
                </button>
              )}

              {draft.preserved.length ? (
                <p className="type-secondary text-text-tertiary">
                  {translateUi(
                    "{{count}} advanced rules set elsewhere are kept and checked first.",
                    {
                      count: draft.preserved.length,
                    },
                  )}
                </p>
              ) : null}
              {draft.keyIds.length > 0 && (
                <p className="type-secondary text-text-tertiary">
                  {translateUi(
                    "The live assistant turn's own stateless model call keeps its existing approval exemption.",
                  )}
                </p>
              )}
              {tooManyRules ? (
                <p
                  role="alert"
                  className="type-validation-error text-destructive"
                >
                  {translateUi(
                    "A service can hold {{max}} rules. Set fewer actions individually.",
                    {
                      max: MAX_PROTECTION_RULES,
                    },
                  )}
                </p>
              ) : null}
            </>
          ) : null}

          {at === "actions" && customising ? (
            <>
              <SearchField
                label={translateUi("Search {{count}} actions", {
                  count: actions.length,
                })}
                value={actionQuery}
                onChange={setActionQuery}
              />
              <p className="type-secondary text-text-tertiary">
                {draft.keyIds.length
                  ? translateUi(
                      "Ask asks every time. Once is not available when the protection is limited to specific keys.",
                    )
                  : translateUi(
                      "Ask asks every time. Once asks once, then remembers your approval for {{count}} days.",
                      {
                        count: days,
                      },
                    )}
              </p>
              <ul className="divide-y divide-border/50 rounded-lg border border-border/50">
                {visibleActions.map((action) => {
                  const verb = actionVerb(action.method);
                  return (
                    <li
                      key={action.id}
                      className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2"
                    >
                      <div className="min-w-0 flex-1 basis-40">
                        <p className="flex flex-wrap items-center gap-1.5 type-row-title text-foreground">
                          <span className="break-words">{action.name}</span>
                          {verb === "destructive" ? (
                            <Badge variant="secondary">
                              {translateUi("Deletes")}
                            </Badge>
                          ) : verb === "write" ? (
                            <Badge variant="secondary">
                              {translateUi("Changes")}
                            </Badge>
                          ) : null}
                        </p>
                        <p className="break-all type-code text-text-tertiary">
                          {action.method.toUpperCase()} {action.path}
                        </p>
                      </div>
                      <ActionChoiceGroup
                        label={action.name}
                        value={effectiveSetting(draft, action)}
                        onChange={(setting) =>
                          setEdited(withActionSetting(draft, action, setting))
                        }
                        disabledOptions={
                          draft.keyIds.length ? ["ask_once"] : []
                        }
                      />
                    </li>
                  );
                })}
                {visibleActions.length === 0 ? (
                  <li className="px-3 py-4 text-center type-secondary text-text-tertiary">
                    {translateUi("No action matches your search.")}
                  </li>
                ) : null}
              </ul>
              {tooManyRules ? (
                <p
                  role="alert"
                  className="type-validation-error text-destructive"
                >
                  {translateUi(
                    "A service can hold {{max}} rules. Set fewer actions individually.",
                    {
                      max: MAX_PROTECTION_RULES,
                    },
                  )}
                </p>
              ) : null}
            </>
          ) : null}

          {at === "where" ? (
            <>
              <WhereControls
                settings={settings}
                value={where}
                onChange={(changes) => setDeliveryEdits((previous) => ({ ...previous, ...changes }))}
                disabled={busy}
              />
              <p className="type-secondary text-text-tertiary">
                {translateUi(
                  "This applies to all of your protections, not only this service.",
                )}
              </p>
            </>
          ) : null}

          {at === "review" && service ? (
            <dl className="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-2.5 type-body">
              <dt className="text-text-tertiary">{translateUi("Service")}</dt>
              <dd className="text-foreground">{service.label}</dd>
              <dt className="text-text-tertiary">
                {translateUi("Applies to")}
              </dt>
              <dd className="text-foreground">{scopeLabel}</dd>
              <dt className="text-text-tertiary">
                {translateUi("Require approval for")}
              </dt>
              <dd className="text-foreground">
                {translateUi(BASE_LABELS[draft.base])}
              </dd>
              {draft.base === "allow_all" ? null : (
                <>
                  <dt className="text-text-tertiary">
                    {translateUi("How often")}
                  </dt>
                  <dd className="text-foreground">
                    {!draft.remember
                      ? translateUi("Every time")
                      : days === 1
                        ? translateUi("Ask again after 1 day")
                        : translateUi("Ask again after {{count}} days", {
                            count: days,
                          })}
                  </dd>
                </>
              )}
              {(["allow", "ask", "block"] as const).map((choice) => {
                const rows = picked.filter(
                  (row) => row.setting.choice === choice,
                );
                return rows.length ? (
                  <div key={choice} className="contents">
                    <dt className="text-text-tertiary">
                      {translateUi(CHOICE_LABELS[choice])}
                    </dt>
                    <dd className="space-y-0.5 text-foreground">
                      {rows.map((row) => (
                        <p key={row.action.id} className="break-words">
                          {row.action.name}
                          {choice === "ask" ? (
                            <span className="text-text-tertiary">
                              {" · "}
                              {row.setting.everyTime
                                ? translateUi("every time")
                                : translateUi("remembered")}
                            </span>
                          ) : null}
                        </p>
                      ))}
                    </dd>
                  </div>
                ) : null;
              })}
              <dt className="text-text-tertiary">{translateUi("Asked in")}</dt>
              <dd className="text-foreground">{whereSummary(where)}</dd>
              <dt className="text-text-tertiary">{translateUi("No answer")}</dt>
              <dd className="text-foreground">
                {translateUi("Rejected after {{time}}", {
                  time: timeoutLabel(settings.approval_timeout_secs),
                })}
              </dd>
            </dl>
          ) : null}
        </DialogBody>

        {at === "actions" && customising ? (
          <DialogFooter>
            <Button variant="primary" onClick={() => setCustomising(false)}>
              {translateUi("Done")}
            </Button>
          </DialogFooter>
        ) : (
          <DialogFooter className="md:justify-between">
            <div className="flex gap-2">
              {step > 1 ? (
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => setStep(step - 1)}
                >
                  {translateUi("Back")}
                </Button>
              ) : null}
              {editing ? (
                <Button
                  variant="destructive"
                  disabled={busy}
                  onClick={() => void removeProtection()}
                >
                  {translateUi("Remove protection")}
                </Button>
              ) : null}
            </div>
            {step < STEPS.length ? (
              <Button
                variant="primary"
                disabled={
                  !service ||
                  (at === "keys" && limited && draft.keyIds.length === 0) ||
                  (at === "actions" &&
                    (tooManyRules ||
                      actionsLoading ||
                      endpoints.isError ||
                      !safeEdit))
                }
                onClick={() => setStep(step + 1)}
              >
                {translateUi("Continue")}
              </Button>
            ) : (
              <Button
                variant="primary"
                disabled={busy || tooManyRules || !safeEdit || !keyRulesReady}
                onClick={() =>
                  overridesGeneral ? setConfirmingOverride(true) : void turnOn()
                }
              >
                {editing ? translateUi("Save changes") : translateUi("Turn on")}
              </Button>
            )}
          </DialogFooter>
        )}
      </DialogContent>

      <Dialog open={confirmingOverride} onOpenChange={setConfirmingOverride}>
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {translateUi("Agent key rules take priority")}
            </DialogTitle>
            <DialogDescription>
              {translateUi(
                "{{service}} has rules for all keys and rules for specific agent keys. A key with its own rules follows only those and ignores the All keys rules, because they are more specific.",
                {
                  service: service?.label ?? "",
                },
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogBody className="py-3">
            <ul className="space-y-1 type-secondary text-text-tertiary">
              {nextPolicy.keyed.map((entry) => (
                <li key={entry.keyIds.join(",")}>
                  <span className="text-foreground">
                    {keyNames(entry.keyIds, keys).join(", ")}
                  </span>
                  {" · "}
                  {translateUi(BASE_LABELS[entry.base])}
                </li>
              ))}
              {nextPolicy.all ? (
                <li>
                  <span className="text-foreground">
                    {translateUi("Every other key")}
                  </span>
                  {" · "}
                  {translateUi(BASE_LABELS[nextPolicy.all.base])}
                </li>
              ) : null}
            </ul>
          </DialogBody>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setConfirmingOverride(false)}
            >
              {translateUi("Go back")}
            </Button>
            <Button
              variant="primary"
              disabled={busy || !keyRulesReady}
              onClick={() => void turnOn()}
            >
              {editing ? translateUi("Save changes") : translateUi("Turn on")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Dialog>
  );
}

function RememberedApprovals() {
  const grants = useApprovalGrants(1, 50);
  const revoke = useRevokeGrant();
  const remembered = (grants.data?.grants ?? []).filter(
    (grant) => !grant.execution && !grant.org_scoped,
  );
  if (!remembered.length) return null;
  return (
    <GroupedList
      title={translateUi("Remembered approvals")}
      footer={translateUi(
        "These run without asking until they expire. Revoke one to be asked again.",
      )}
    >
      {remembered.map((grant) => (
        <GroupedRow
          key={grant.id}
          title={grant.service_name}
          description={translateUi("{{requester}} · until {{date}}", {
            requester: grant.requester_label ?? grant.requester_type,
            date: new Date(grant.expires_at).toLocaleDateString(
              getFormattingLocale(),
              {
                year: "numeric",
                month: "short",
                day: "numeric",
              },
            ),
          })}
          accessory={
            <Button
              variant="outline"
              size="sm"
              disabled={revoke.isPending}
              aria-label={translateUi("Revoke grant for {{value1}}", {
                value1: grant.service_name,
              })}
              onClick={() =>
                revoke.mutate(
                  { grantId: grant.id },
                  {
                    onSuccess: () =>
                      toast.success(translateUi("Grant revoked")),
                    onError: (error) =>
                      toast.error(
                        translateErrorMessage(
                          errorText(error, "Failed to revoke grant"),
                        ),
                      ),
                  },
                )
              }
            >
              {translateUi("Revoke")}
            </Button>
          }
        />
      ))}
    </GroupedList>
  );
}

/** Rules tab of the Approvals page: what needs approval, and where requests arrive. */
export function ApprovalRules() {
  useUiTranslation();
  const settings = useNotificationSettings();
  const userServices = useUserServices();
  const configs = useServiceApprovalConfigs();
  const updateSettings = useUpdateNotificationSettings();
  const apiKeys = useApiKeys();
  const capabilities = useApprovalAuthoringCapabilities();
  const [storedEditor, setStoredEditor] =
    useState<ServiceApprovalConfigItem | null>(null);
  const [setup, setSetup] = useState<{
    readonly initial: ProtectionScope | null;
  } | null>(null);

  const services = useMemo((): readonly ProtectableService[] => {
    const stored = configs.data?.configs ?? [];
    const dominated = new Set(
      (configs.data?.dominant_org_policies ?? []).map(
        (policy) => `${policy.org_id}|${policy.service_id}`,
      ),
    );
    const configFor = (ids: readonly (string | null | undefined)[]) =>
      stored.find(
        (config) =>
          ids.includes(config.service_id) ||
          (config.user_service_id !== undefined &&
            ids.includes(config.user_service_id)),
      ) ?? null;
    const out: ProtectableService[] = [];
    const seen = new Set<string>();
    for (const item of userServices.data ?? []) {
      if (!item.is_active) continue;
      const source = item.credential_source;
      const catalogId = item.catalog_service_id ?? null;
      let id: string | null = null;
      let detail = translateUi("Personal connection");
      if (source.type === "personal") {
        id = item.id;
      } else if (source.type === "org" && source.allowed && catalogId) {
        if (dominated.has(`${source.org_id}|${catalogId}`)) continue;
        id = catalogId;
        detail = translateUi("Shared by {{org}}", { org: source.org_name });
      }
      const key = catalogId ?? id;
      if (!id || !key || seen.has(key)) continue;
      seen.add(key);
      out.push({
        id,
        label: item.catalog_service_name ?? item.label ?? item.slug,
        detail: `${detail} · ${item.slug}`,
        slug: item.slug,
        catalogServiceId: catalogId,
        config: configFor([item.id, catalogId]),
      });
    }
    return out;
  }, [userServices.data, configs.data]);

  const protections = useMemo((): readonly {
    readonly service: ProtectableService;
    readonly draft: ProtectionDraft;
  }[] => {
    const known = new Set(
      services.flatMap((entry) => (entry.config ? [entry.config] : [])),
    );
    const orphans = (configs.data?.configs ?? [])
      .filter((config) => !known.has(config))
      .map((config) => ({
        id: config.user_service_id ?? config.service_id,
        label: config.service_name,
        detail: translateUi("Connection no longer active"),
        slug: config.user_service_slug ?? "",
        catalogServiceId: null,
        config,
      }));
    const seen = new Set<string>();
    return [...services.filter((entry) => entry.config), ...orphans]
      .filter((service) => {
        const id = service.config!.service_id;
        if (seen.has(id)) return false;
        seen.add(id);
        return true;
      })
      .map((service) => ({ service, draft: newProtection([]) }));
  }, [services, configs.data]);

  if (settings.isLoading || configs.isLoading || userServices.isLoading) {
    return (
      <div
        role="status"
        aria-label={translateUi("Loading approval rules")}
        className="space-y-3"
      >
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (!settings.data || configs.isError || userServices.isError) {
    return (
      <GroupedList footer={translateUi("Approval rules could not be loaded.")}>
        <GroupedRow asChild interactive>
          <button
            onClick={() => {
              void settings.refetch();
              void configs.refetch();
              void userServices.refetch();
            }}
          >
            {translateUi("Retry")}
          </button>
        </GroupedRow>
      </GroupedList>
    );
  }

  const current = settings.data;
  const globalMode: GlobalApprovalMode =
    current.global_approval_mode ?? (current.approval_required ? "all" : "off");

  function update(
    data: Parameters<typeof updateSettings.mutate>[0],
    failure: string,
  ) {
    updateSettings.mutate(data, {
      onError: (error) =>
        toast.error(translateErrorMessage(errorText(error, failure))),
    });
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3 px-4">
        <p className="min-w-0 flex-1 basis-56 type-secondary text-text-tertiary">
          {translateUi(
            "Choose which actions wait for your approve or reject, one service at a time.",
          )}
        </p>
        <Button variant="primary" onClick={() => setSetup({ initial: null })}>
          {translateUi("Set up a protection")}
        </Button>
      </div>

      {protections.length ? (
        <GroupedList title={translateUi("Protections")}>
          {protections.map(({ service }) => (
            <GroupedRow key={service.config!.service_id} asChild interactive>
              <button
                type="button"
                aria-label={translateUi(
                  "Edit {{service}} protection for {{scope}}",
                  {
                    service: service.label,
                    scope:
                      service.config?.rules.length &&
                      service.config.rules.every(
                        (rule) => (rule.api_key_ids?.length ?? 0) > 0,
                      ) &&
                      !service.config.default_effect
                        ? keyNames(
                            [
                              ...new Set(
                                service.config.rules.flatMap(
                                  (rule) => rule.api_key_ids ?? [],
                                ),
                              ),
                            ],
                            apiKeys.data ?? [],
                          ).join(", ")
                        : translateUi("All keys"),
                  },
                )}
                onClick={() => setStoredEditor(service.config)}
              >
                <ServiceIcon slug={service.slug} size="sm" />
                <span className="min-w-0 flex-1 basis-32 break-words">
                  <span className="block type-row-title text-foreground">
                    {service.label}
                  </span>
                  <span className="block type-secondary text-text-tertiary">
                    {service.detail}
                  </span>
                  <span className="block type-secondary text-text-tertiary">
                    {translateUi(
                      service.config?.disabled ? "Disabled" : "Enabled",
                    )}{" "}
                    ·{" "}
                    {translateUi("{{count}} ordered rules", {
                      count: service.config?.rules.length ?? 0,
                    })}
                  </span>
                </span>
                <span className="flex shrink-0 items-center gap-1 type-secondary text-text-tertiary">
                  {translateUi("Edit")}
                  <ChevronRight aria-hidden className="size-4" />
                </span>
              </button>
            </GroupedRow>
          ))}
        </GroupedList>
      ) : (
        <div className="flex flex-col items-center rounded-lg border border-dashed border-border px-4 py-10 text-center">
          <ShieldCheck aria-hidden className="mb-3 size-6 text-text-tertiary" />
          <p className="type-card-title">{translateUi("No protections yet")}</p>
          <p className="mt-1 max-w-sm type-secondary text-text-tertiary">
            {translateUi(
              "Pick a service and the actions that should wait for you. It takes about a minute.",
            )}
          </p>
        </div>
      )}

      <RememberedApprovals />

      <section className="space-y-2">
        <h3 className="px-4 type-group-heading text-text-tertiary">
          {translateUi("Where you are asked")}
        </h3>
        <WhereControls
          settings={current}
          value={{
            push: current.push_enabled,
            telegram: current.telegram_enabled,
          }}
          disabled={updateSettings.isPending}
          onChange={(where) =>
            update(
              { push_enabled: where.push, telegram_enabled: where.telegram },
              "Failed to update where you are asked",
            )
          }
        />
      </section>

      <GroupedList
        title={translateUi("Everything else")}
        footer={translateUi(
          "Applies to services without a protection. Unanswered requests are rejected after {{time}}.",
          {
            time: timeoutLabel(current.approval_timeout_secs),
          },
        )}
      >
        <GroupedRow
          title={translateUi("Ask before requests run")}
          description={
            globalMode === "all"
              ? translateUi("Every request waits for you, including reads.")
              : globalMode === "off"
                ? translateUi(
                    "Nothing waits for you unless a protection says so.",
                  )
                : translateUi("Reads run on their own. Changes wait for you.")
          }
          accessory={
            <SegmentedControl
              value={globalMode}
              onValueChange={(value) =>
                update(
                  { global_approval_mode: value as GlobalApprovalMode },
                  "Failed to update the default",
                )
              }
            >
              <SegmentedControlList
                aria-label={translateUi("Ask before requests run")}
                className="w-auto flex-nowrap"
              >
                <SegmentedControlTrigger
                  value="off"
                  disabled={updateSettings.isPending}
                >
                  {translateUi("Off")}
                </SegmentedControlTrigger>
                <SegmentedControlTrigger
                  value="writes_only"
                  disabled={updateSettings.isPending}
                >
                  {translateUi("Changes")}
                </SegmentedControlTrigger>
                <SegmentedControlTrigger
                  value="all"
                  disabled={updateSettings.isPending}
                >
                  {translateUi("All requests")}
                </SegmentedControlTrigger>
              </SegmentedControlList>
            </SegmentedControl>
          }
        />
      </GroupedList>

      {storedEditor && (
        <StoredApprovalPolicyDialog
          config={storedEditor}
          apiKeys={apiKeys.data ?? []}
          onClose={() => setStoredEditor(null)}
        />
      )}
      {setup ? (
        <ProtectionSetupDialog
          services={services}
          onEditStored={(config) => {
            setSetup(null);
            setStoredEditor(config);
          }}
          initial={setup.initial}
          settings={current}
          rulesReady={
            capabilities.data?.personal_rules === true && !capabilities.error
          }
          onClose={() => setSetup(null)}
        />
      ) : null}
    </div>
  );
}
