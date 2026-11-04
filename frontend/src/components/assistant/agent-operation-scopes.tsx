import { DrawerSearchRevealContext } from "@/lib/drawer-search";
import { SectionInfo } from "@/components/ui/section-info";
import { describeChanges, changeList } from "@/lib/grouped-form-changes";
import { CompactScroll } from "@/components/ui/compact-title";
import { useDrawerSearchEntries, useDrawerSearchPage } from "@/lib/drawer-search";
import {
  GroupedFormActions,
  GroupedFormScope,
  GroupedFormBar,
} from "@/components/ui/grouped-form";
import { useContext, useEffect, useId, useRef, useState } from "react";
import { GroupedDraftOwnerContext, useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { createPortal } from "react-dom";
import { coverGroupedContent } from "@/lib/grouped-detail";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronLeft, X } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedDisclosure,
  GroupedInput,
  GroupedSelectTrigger,
} from "@/components/ui/grouped-list";
import { useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import {
  useAgentOperations,
  useSetAgentOperations,
} from "@/hooks/use-agent-operation-scopes";
import {
  operationSelectionSchema,
  type AgentServiceOperations,
  type OperationSelection,
} from "@/schemas/agent-operation-scopes";

export function AgentOperationScopes({
  agentId,
  disabled = false,
  guests = false,
  drillIn = false,
  visible = true,
  onClose,
  detailContainer,
}: {
  readonly agentId: string;
  readonly disabled?: boolean;
  readonly guests?: boolean;
  readonly drillIn?: boolean;
  readonly visible?: boolean;
  readonly onClose?: () => void;
  readonly detailContainer?: HTMLElement | null;
}) {
  const [active, setActive] = useState<string>();
  const guard = useGroupedDraftGuard();
  const parentOwner = useContext(GroupedDraftOwnerContext);
  const owner = `${parentOwner}/${useId()}`;
  const [visited, setVisited] = useState<string[]>([]);
  const root = useRef<HTMLDivElement>(null);
  const views = useRef(new Map<string, HTMLElement>());
  const returnFocus = useRef<HTMLButtonElement>(null);
  const enabled = useFeature(guests ? FEATURE_FLAG.GUEST_OPERATION_SCOPES : FEATURE_FLAG.AGENT_OPERATION_SCOPES);
  const query = useAgentOperations(agentId, enabled && !disabled, guests);
  const searchId = (id: string) => `operations-${guests ? "guest-" : ""}${id}`;
  useDrawerSearchEntries(enabled && drillIn ? (query.data ?? []).map((service) => ({
    id: searchId(service.service_id), title: `${service.service_name} · ${guests ? "Guest operations" : "Operations"}`,
    breadcrumb: `Access › ${guests ? "Guest service operations" : "Service operations"}`,
    keywords: `${service.service_slug} ${service.all_operations ? "All operations" : "Selected operations"} ${service.operations.map((operation) => `${operation.summary ?? ""} ${operation.method} ${operation.path}`).join(" ")} ${service.rules.map((rule) => `${rule.method} ${rule.path_template}`).join(" ")}`,
    pages: [searchId(service.service_id)], reveal: () => {
      returnFocus.current = [...(root.current?.querySelectorAll<HTMLButtonElement>("[data-operation-link]") ?? [])].find((element) => element.dataset.operationLink === service.service_id) ?? null;
      setVisited((current) => current.includes(service.service_id) ? current : [...current, service.service_id]);
      setActive(service.service_id);
    },
  })) : []);
  useDrawerSearchPage(active ? { id: searchId(active), open: true, leave: async () => {
    if (guard && !await guard.confirmDiscard(`${owner}/${active}`)) return false;
    setActive(undefined); return true;
  } } : null);
  const activeAvailable = Boolean(
    query.data?.some((service) => service.service_id === active),
  );
  useEffect(() => {
    if (!enabled || !visible) return;
    if (!active) {
      returnFocus.current?.focus({ preventScroll: true });
      return;
    }
    const view = activeAvailable ? views.current.get(active) : undefined;
    if (!view) return;
    // Covered sheet controls leave keyboard and screen-reader navigation until Back.
    const release: (() => void)[] = [];
    const boundary = detailContainer ?? root.current?.closest('[role="dialog"]') ?? root.current;
    let current: HTMLElement = view;
    while (current !== boundary && current.parentElement) {
      const parent = current.parentElement;
      for (const sibling of parent.children) {
        if (sibling === current || !(sibling instanceof HTMLElement)) continue;
        release.push(coverGroupedContent(sibling));
      }
      if (parent.getAttribute("role") === "dialog") break;
      current = parent;
    }
    view.querySelector<HTMLElement>("h2")?.focus({ preventScroll: true });
    return () => {
      release.forEach((restore) => restore());
      if (!view.isConnected) {
        const target = returnFocus.current?.isConnected
          ? returnFocus.current
          : boundary?.querySelector<HTMLElement>(
              '[role="tab"][aria-selected="true"]',
            );
        target?.focus({ preventScroll: true });
      }
    };
  }, [active, activeAvailable, enabled, visible, detailContainer]);
  if (!enabled) {
    return (
      <GroupedList
        title={guests ? "Guest service operations" : "Service operations"}
        footer={
          <p role="status">
            {guests ? "Guest operation" : "Operation scope"} configuration is
            not enabled yet. Existing operation limits still apply.
          </p>
        }
      >
        <GroupedRow title="Configuration unavailable" />
      </GroupedList>
    );
  }
  return (
    <div
      ref={root}
      className="space-y-3"
      aria-label={guests ? "Guest service operations" : "Service operations"}
    >
      <GroupedList
        title={guests ? "Guest service operations" : "Service operations"}
        footer={
          guests
            ? "For guest turns only: withhold a service or select its permitted operations. Your own turns keep the service. The guest level above, specialist limits and confirmations still apply."
            : "Limit each granted service to the operations this specialist needs. Guest access and confirmations still apply."
        }
      >
        {query.isLoading ? <GroupedRow title="Loading operations…" /> : null}
        {query.error ? (
          <GroupedRow>
            <p role="alert" className="text-destructive">
              Could not load operation scopes.
            </p>
          </GroupedRow>
        ) : null}
        {drillIn
          ? query.data?.map((service) => (
              <GroupedDisclosure
                key={service.service_id}
                data-operation-link={service.service_id}
                onClick={(event) => {
                  returnFocus.current = event.currentTarget;
                  setVisited((current) =>
                    current.includes(service.service_id)
                      ? current
                      : [...current, service.service_id],
                  );
                  setActive(service.service_id);
                }}
              >
                <span className="block font-medium">
                  {service.service_name}
                </span>
                <span className="text-11 font-normal text-text-tertiary">
                  {service.all_operations
                    ? "All operations"
                    : `${service.endpoint_ids.length + service.rules.length} selected operations`}
                </span>
              </GroupedDisclosure>
            ))
          : null}
        {drillIn && !query.isLoading && !query.error && !query.data?.length ? (
          <GroupedRow title="No granted services" />
        ) : null}
      </GroupedList>
      {query.data?.map((service) => {
        if (!drillIn)
          return (
            <ServiceOperationForm
              key={`${service.service_id}:${service.revision}`}
              agentId={agentId}
              guests={guests}
              service={service}
              disabled={disabled}
            />
          );
        if (!visited.includes(service.service_id)) return null;
        const view = (
          <section
            key={service.service_id}
            ref={(element) => {
              if (element) views.current.set(service.service_id, element);
              else views.current.delete(service.service_id);
            }}
            hidden={!visible || active !== service.service_id}
            style={
              !visible || active !== service.service_id
                ? { display: "none" }
                : undefined
            }
            data-operation-view={service.service_id}
            data-grouped-level={1}
            aria-label={`${service.service_name} ${guests ? "guest " : ""}operation details`}
            className="absolute inset-0 z-10 flex flex-col bg-background p-4"
          >
            <div className="relative shrink-0 pb-4 pr-6">
              {onClose ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label="Close agent details"
                  className="absolute right-0 top-0"
                  onClick={onClose}
                >
                  <X aria-hidden className="size-[1.125em]" />
                </Button>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={async () => {
                  if (!guard || await guard.confirmDiscard(`${owner}/${service.service_id}`)) setActive(undefined);
                }}
              >
                <ChevronLeft aria-hidden />
                Back to Access
              </Button>
              <h2
                tabIndex={-1}
                className="mt-3 flex items-center gap-1 break-words px-4 text-15 font-semibold outline-none"
              >
                {service.service_name}
                {guests ? " · Guest operations" : " · Operations"}
                <SectionInfo title={`${service.service_name} · ${guests ? "Guest operations" : "Operations"}`}>Choose the allowed operations for this service. Permissions remain limited by the agent’s grant and save against the current service revision.</SectionInfo>
              </h2>
            </div>
            <CompactScroll active={visible && active === service.service_id} level={1} className="assistant-scrollbar flex-1">
              <GroupedFormScope inherit>
                <GroupedDraftOwnerContext.Provider value={`${owner}/${service.service_id}`}>
                <DrawerSearchRevealContext value={{ pages: [searchId(service.service_id)], reveal: () => setActive(service.service_id) }}>
                <ServiceOperationForm
                  key={`${service.service_id}:${service.revision}`}
                  agentId={agentId}
                  guests={guests}
                  service={service}
                  disabled={disabled}
                />
                </DrawerSearchRevealContext></GroupedDraftOwnerContext.Provider>
                <GroupedFormBar />
              </GroupedFormScope>
            </CompactScroll>
          </section>
        );
        return detailContainer
          ? createPortal(view, detailContainer, service.service_id)
          : view;
      })}
    </div>
  );
}

export function ServiceOperationForm({
  agentId,
  service,
  disabled = false,
  guests = false,
}: {
  readonly agentId: string;
  readonly service: AgentServiceOperations;
  readonly disabled?: boolean;
  readonly guests?: boolean;
}) {
  const mutation = useSetAgentOperations(agentId, guests);
  return (
    <OperationSelectionForm
      service={service}
      guests={guests}
      disabled={disabled}
      isSaving={mutation.isPending}
      save={(selection) =>
        mutation.mutateAsync({ serviceId: service.service_id, selection })
      }
    />
  );
}

/** One service's operation picker; the holder (specialist or Agent Key) supplies `save`. */
export function OperationSelectionForm({
  service,
  disabled = false,
  isSaving,
  save: persist,
  guests = false,
}: {
  readonly service: AgentServiceOperations;
  readonly disabled?: boolean;
  readonly isSaving: boolean;
  readonly guests?: boolean;
  readonly save: (selection: OperationSelection) => Promise<unknown>;
}) {
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string>();
  const form = useAppForm<OperationSelection>({
    resolver: zodResolver(operationSelectionSchema),
    defaultValues: {
      expected_revision: service.revision,
      all_operations: service.all_operations,
      endpoint_ids: service.endpoint_ids,
      rules: service.rules,
    },
  });
  const savedInputs = service.inputs ?? {};
  const all = form.watch("all_operations");
  const selected = form.watch("endpoint_ids");
  const rules = form.watch("rules");
  const [guestMode, setGuestMode] = useState(service.all_operations ? "all" : service.endpoint_ids.length ? "selected" : "none");
  const operations = service.operations.filter((op) =>
    `${op.method} ${op.path} ${op.summary ?? ""}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  async function save(selection: OperationSelection) {
    setError(undefined);
    try {
      const next = selection.all_operations
        ? { ...selection, endpoint_ids: [], rules: [] }
        : withSavedInputs(selection, savedInputs);
      await persist(next);
      form.reset(next);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save operations. Reload and try again.",
      );
    }
  }
  return (
    <form
      aria-label={`${service.service_name} ${guests ? "guest " : ""}operations`}
      onSubmit={form.handleSubmit(save)}
      className="space-y-6"
    >
      <GroupedList
        footer={
          <>
            <p>Revision {service.revision}</p>
            {error ? (
              <p role="alert" className="text-destructive">
                {error}
              </p>
            ) : null}
            {Object.keys(form.formState.errors).length ? (
              <p role="alert" className="text-destructive">
                Check the selection: paths must start with / and at most 256
                operations may be selected.
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow data-drawer-search-target={`operations-${guests ? "guest-" : ""}${service.service_id}`}>
          <span className="mr-auto">Allowed operations</span>
          <Select
            value={guests ? guestMode : all ? "all" : "selected"}
            disabled={disabled}
            onValueChange={(value) => {
              setGuestMode(value);
              form.setValue("all_operations", value === "all");
              if (value === "none") {
                form.setValue("endpoint_ids", []);
                form.setValue("rules", []);
              }
            }}
          >
            <GroupedSelectTrigger
              aria-label={`${service.service_name} ${guests ? "guest " : ""}operation access`}
            >
              <SelectValue />
            </GroupedSelectTrigger>
            <SelectContent>
              {guests ? (
                <SelectItem value="none">No guest access</SelectItem>
              ) : null}
              <SelectItem value="all">
                {guests
                  ? "All operations permitted by guest level"
                  : "All operations"}
              </SelectItem>
              <SelectItem value="selected">Selected operations</SelectItem>
            </SelectContent>
          </Select>
        </GroupedRow>
      </GroupedList>
      {!all && (!guests || guestMode === "selected") ? (
        <>
          {service.operations.length ? (
            <>
              <GroupedList
                title="Select operations"
                footer="An empty selection blocks every operation."
              >
                <GroupedRow>
                  <GroupedInput
                    aria-label={`Search ${service.service_name} operations`}
                    placeholder="Search method, path or summary"
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                  />
                </GroupedRow>
                <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-2">
                  <span className="text-11 text-text-tertiary">
                    {selected.length} selected
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    disabled={disabled}
                    onClick={() =>
                      form.setValue(
                        "endpoint_ids",
                        service.operations
                          .filter((op) => op.read_only && !op.changes_existing)
                          .map((op) => op.endpoint_id),
                      )
                    }
                  >
                    Select all reads
                  </Button>
                </div>
                <div className="assistant-scrollbar max-h-96 overflow-auto">
                  {operations.map((operation) => (
                    <GroupedRow
                      key={operation.endpoint_id}
                      asChild
                      interactive
                      className="items-start"
                    >
                      <label>
                        <Checkbox
                          checked={selected.includes(operation.endpoint_id)}
                          disabled={disabled}
                          aria-label={`${operation.method} ${operation.path}`}
                          onCheckedChange={(checked) =>
                            form.setValue(
                              "endpoint_ids",
                              checked
                                ? [...selected, operation.endpoint_id]
                                : selected.filter(
                                    (id) => id !== operation.endpoint_id,
                                  ),
                            )
                          }
                        />
                        <span className="min-w-0 flex-1 space-y-1">
                          <span className="block break-all font-mono">
                            {operation.method} {operation.path}
                          </span>
                          <span className="block text-11 font-normal text-text-tertiary">
                            {operation.summary}
                          </span>
                          <span className="flex gap-1">
                            {operation.read_only ? (
                              <span className="text-11 font-normal text-text-tertiary">
                                Read only
                              </span>
                            ) : null}
                            {operation.changes_existing ? (
                              <span className="text-11 font-normal text-warning">
                                Changes existing
                              </span>
                            ) : null}
                            {savedInputs[operation.endpoint_id] ? (
                              <span className="text-11 font-normal text-text-tertiary">
                                Value limits
                              </span>
                            ) : null}
                          </span>
                        </span>
                      </label>
                    </GroupedRow>
                  ))}
                  {!operations.length ? (
                    <GroupedRow title="No matching operations." />
                  ) : null}
                </div>
              </GroupedList>
            </>
          ) : service.allows_explicit_rules ? (
            <>
              <GroupedList
                title="Custom operations"
                footer={
                  <>
                    No catalog operations. Add exact methods and paths; a
                    variable such as {"{id}"} matches one segment. An empty
                    selection blocks every operation.
                  </>
                }
              >
                <GroupedRow title="Use exact methods and paths" />
              </GroupedList>
              {rules.map((rule, index) => (
                <GroupedList key={index} title={`Rule ${index + 1}`}>
                  <GroupedRow>
                    <Select
                      value={rule.method}
                      disabled={disabled}
                      onValueChange={(method) =>
                        form.setValue(
                          `rules.${index}.method`,
                          method as typeof rule.method,
                        )
                      }
                    >
                      <GroupedSelectTrigger
                        className="w-auto"
                        aria-label={`Rule ${index + 1} method`}
                      >
                        <SelectValue />
                      </GroupedSelectTrigger>
                      <SelectContent>
                        {[
                          "GET",
                          "HEAD",
                          "OPTIONS",
                          "POST",
                          "PUT",
                          "PATCH",
                          "DELETE",
                        ].map((method) => (
                          <SelectItem key={method} value={method}>
                            {method}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <GroupedInput
                      aria-label={`Rule ${index + 1} path`}
                      {...form.register(`rules.${index}.path_template`)}
                      disabled={disabled}
                    />
                    <Button
                      variant="ghost"
                      type="button"
                      size="sm"
                      disabled={disabled}
                      onClick={() =>
                        form.setValue(
                          "rules",
                          rules.filter((_, i) => i !== index),
                        )
                      }
                    >
                      Remove
                    </Button>
                  </GroupedRow>
                </GroupedList>
              ))}
              <GroupedList>
                <GroupedRow asChild interactive>
                  <button
                    type="button"
                    disabled={disabled || rules.length >= 256}
                    onClick={() =>
                      form.setValue("rules", [
                        ...rules,
                        { method: "GET", path_template: "/" },
                      ])
                    }
                  >
                    Add rule
                  </button>
                </GroupedRow>
              </GroupedList>
            </>
          ) : (
            <GroupedList>
              <GroupedRow title="No active catalog operations." />
            </GroupedList>
          )}
          {selected.some(
            (id) => !service.operations.some((op) => op.endpoint_id === id),
          ) ? (
            <GroupedList footer="Some selected operations are unavailable.">
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  onClick={() =>
                    form.setValue(
                      "endpoint_ids",
                      selected.filter((id) =>
                        service.operations.some((op) => op.endpoint_id === id),
                      ),
                    )
                  }
                  disabled={disabled}
                >
                  Remove unavailable selections
                </button>
              </GroupedRow>
            </GroupedList>
          ) : null}
        </>
      ) : null}
      <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "all_operations", label: "Allow all operations" }, { key: "endpoint_ids", label: "Allowed operations", list: (value) => changeList(value).map((id) => { const op = service.operations.find((op) => op.endpoint_id === id); return op ? `${op.method} ${op.path}` : id; }) }, { key: "rules", label: "Custom operations", list: (value) => Array.isArray(value) ? value.map((rule: { method: string; path_template: string }) => `${rule.method} ${rule.path_template}`) : [] }])}
        label={guests ? "guest operations" : "operations"}
        dirty={form.formState.isDirty}
        pending={isSaving}
        disabled={disabled}
        saveLabel={guests ? "Save guest operations" : "Save operations"}
        save={form.handleSubmit(save)}
        discard={() => {
          form.reset();
          setGuestMode(service.all_operations ? "all" : service.endpoint_ids.length ? "selected" : "none");
          setError(undefined);
        }}
      />
    </form>
  );
}

/** Keep saved value limits for operations that stay selected; deselecting drops them. */
function withSavedInputs(
  selection: OperationSelection,
  saved: Readonly<Record<string, unknown>>,
): OperationSelection {
  const inputs = Object.fromEntries(
    Object.entries(saved).filter(([id]) => selection.endpoint_ids.includes(id)),
  );
  return Object.keys(inputs).length ? { ...selection, inputs } : selection;
}
