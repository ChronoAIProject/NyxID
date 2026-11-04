import { useDrawerSearchEntries } from "@/lib/drawer-search";
import { Search } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedSelectTrigger,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { useState, type ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import {
  useAgentSkills,
  useSetAgentSkills,
  useSkillSearch,
  useSkillVersions,
  useSkillPreview,
  useAttachedSkillPreview,
  useAgentSkillHistory,
  useImproveAgentSkill,
} from "@/hooks/use-agent-skills";
import type {
  AgentSkill,
  AgentSkillHistoryItem,
  AttachedSkillPreview,
} from "@/schemas/agent-skills";
import { ProposalDraftEditor } from "./learning-draft-editor";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { SkillPreloadSettings } from "./skill-preload-settings";

const HISTORY_LABELS: Record<string, string> = {
  assistant_skills_updated: "Attached skills changed",
  assistant_skill_preload_updated: "Skill preload settings changed",
  assistant_learning_proposal_edited: "Skill draft edited",
  assistant_learning_proposal_rejected: "Skill draft rejected",
  assistant_learning_proposal_reprepared: "Skill draft package rebuilt",
  assistant_learning_publication_approved: "Skill publication approved",
  assistant_learning_publication_deferred: "Skill publication deferred",
  assistant_learning_skill_pinned: "Published skill attached",
};

function SkillHistory({
  items,
}: {
  readonly items: readonly AgentSkillHistoryItem[];
}) {
  return (
    <GroupedList title="Activity">
      <ol aria-label="Skill history">
        {[...items]
          .sort(
            (a, b) =>
              (Date.parse(b.created_at) || 0) - (Date.parse(a.created_at) || 0),
          )
          .map((item) => (
            <GroupedRow data-drawer-search-target={`skill-history-${item.id}`} key={item.id} asChild>
              <li>
                <div className="min-w-0 flex-1">
                  <p>{HISTORY_LABELS[item.event_type] ?? "Skill activity"}</p>
                  <p className="text-11 font-normal text-text-tertiary">
                    {new Date(item.created_at).toLocaleString()}
                    {item.revision !== null
                      ? ` · Revision ${item.revision}`
                      : ""}
                  </p>
                  {item.skills.length ? (
                    <p className="text-11 font-normal text-text-tertiary">
                      {item.skills
                        .map(
                          (pin) =>
                            `${pin.name} ${pin.version}${pin.disabled ? " (disabled)" : ""}`,
                        )
                        .join(", ")}
                    </p>
                  ) : null}
                </div>
              </li>
            </GroupedRow>
          ))}
      </ol>
    </GroupedList>
  );
}

/** The outcome of an explicit Test of one pin, read as its agent reads it. */
function TestResult({
  result,
}: {
  readonly result: UseQueryResult<AttachedSkillPreview>;
}) {
  if (result.isFetching) {
    return (
      <p role="status" className="text-11 text-text-tertiary">
        Testing…
      </p>
    );
  }
  if (result.error) {
    return (
      <p role="alert" className="text-11 text-destructive">
        Test failed: {result.error.message}
      </p>
    );
  }
  const data = result.data;
  if (!data) return null;
  if (!data.readable) {
    return (
      <p role="status" className="text-11 text-warning">
        {data.name} {data.version} is disabled, so the agent does not read it.
        Enable it to use it again.
      </p>
    );
  }
  return (
    <div role="status" className="space-y-1">
      <p className="text-11 text-foreground">
        Test passed: the agent reads {data.name} {data.version} (
        {data.size_bytes.toLocaleString()} bytes).
      </p>
      {data.excerpt ? (
        <pre
          aria-label={`${data.name} SKILL.md excerpt`}
          className="max-h-60 overflow-auto whitespace-pre-wrap break-words py-2 text-11"
        >
          {data.excerpt}
        </pre>
      ) : null}
    </div>
  );
}

function AttachedSkill({
  skill,
  description,
  size,
  disabled,
  canDisable,
  canEdit,
  remove,
  review,
  toggle,
  test,
  testResult,
  edit,
  children,
  error,
  open,
  onOpenChange,
}: {
  readonly skill: AgentSkill;
  readonly description?: string;
  readonly size?: number;
  readonly disabled: boolean;
  /** Pin disabling is behind `assistant:skill-pin-disable`; the server refuses it otherwise. */
  readonly canDisable: boolean;
  /** Editing starts an authored improve draft (agent learning). */
  readonly canEdit: boolean;
  readonly remove: () => void;
  readonly review: () => void;
  readonly toggle: (enabled: boolean) => void;
  readonly test: () => void;
  readonly testResult?: UseQueryResult<AttachedSkillPreview>;
  readonly edit: () => void;
  readonly children?: ReactNode;
  readonly error?: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const versions = useSkillVersions(skill.skill_id);
  const latest = versions.data?.items[0]?.version;
  const searchId = `skill-${skill.skill_id}`;
  useDrawerSearchEntries([{ id: searchId, title: skill.name, breadcrumb: `Skills › ${skill.name}`,
    keywords: `${description ?? ""} ${skill.version} ${skill.disabled ? "Disabled" : "Enabled"} preload`,
    pages: [searchId], reveal: () => onOpenChange(true),
  }]);
  return (
    <>
      <GroupedDisclosure
        data-drawer-page-trigger={searchId}
        onClick={() => onOpenChange(true)}
        accessory={
          <span className="font-mono font-normal text-text-tertiary">
            {skill.version}
          </span>
        }
      >
        <span className="min-w-0 flex-1">
          <span className="block">{skill.name}</span>
          <span className="block truncate text-11 font-normal text-text-tertiary" title={description}>
            {skill.disabled ? "Disabled" : description}
          </span>
        </span>
      </GroupedDisclosure>
      <GroupedDetail
        searchPage={searchId}
        open={open}
        title={skill.name}
        backLabel="Back to Skills"
        onBack={() => onOpenChange(false)}
      >
        <GroupedList
          title="Pinned skill"
          footer={
            <>
              {description ? <p>{description}</p> : null}
              {open && error ? (
                <p role="alert" className="text-destructive">
                  {error}
                </p>
              ) : null}
              {size !== undefined ? <p>{size.toLocaleString()} bytes</p> : null}
              {versions.error ? (
                <p>
                  Could not check for updates. Your pinned version is unchanged.
                </p>
              ) : null}
              {testResult ? <TestResult result={testResult} /> : null}
            </>
          }
        >
          <GroupedRow data-drawer-search-target={searchId} title="Version" accessory={skill.version} />
          <GroupedDisclosure disabled={disabled} onClick={review}>
            {latest && latest !== skill.version
              ? `Newer version available: ${latest}`
              : "Choose version"}
          </GroupedDisclosure>
          <GroupedRow asChild interactive>
            <button
              type="button"
              aria-label={`Test ${skill.name}`}
              disabled={testResult?.isFetching}
              onClick={test}
            >
              Test skill
            </button>
          </GroupedRow>
          {canEdit ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                aria-label={`Edit ${skill.name}`}
                disabled={disabled}
                onClick={edit}
              >
                Edit skill
              </button>
            </GroupedRow>
          ) : null}
          {canDisable ? (
            <GroupedRow
              title="Enabled"
              accessory={
                <Switch
                  checked={!skill.disabled}
                  disabled={disabled}
                  aria-label={`Use ${skill.name}`}
                  onCheckedChange={toggle}
                />
              }
            />
          ) : null}
          <GroupedRow asChild interactive className="text-destructive">
            <button type="button" disabled={disabled} onClick={remove}>
              Remove {skill.name}
            </button>
          </GroupedRow>
        </GroupedList>
        {children}
      </GroupedDetail>
    </>
  );
}

export function AgentSkills({
  agentId,
  readOnly = false,
  id,
}: {
  readonly agentId: string;
  readonly readOnly?: boolean;
  readonly id?: string;
}) {
  const current = useAgentSkills(agentId);
  const canDisable = useAuthStore((s) =>
    userHasFeature(s.user, FEATURE_FLAG.SKILL_PIN_DISABLE),
  );
  const canEdit = useAuthStore((s) =>
    userHasFeature(s.user, FEATURE_FLAG.AGENT_LEARNING),
  );
  // Tests run only on an explicit click, one pin at a time.
  const [testing, setTesting] = useState<string>();
  const tested = useAttachedSkillPreview(agentId, testing ?? "");
  const history = useAgentSkillHistory(agentId);
  const write = useSetAgentSkills(agentId);
  const improve = useImproveAgentSkill(agentId);
  const [improving, setImproving] = useState<{
    readonly skillId: string;
    readonly proposalId: string;
  }>();
  const [input, setInput] = useState("");
  const [query, setQuery] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState<{ id: string; name: string }>();
  const [version, setVersion] = useState("");
  const [versionPage, setVersionPage] = useState(1);
  const search = useSkillSearch(query, page);
  const versions = useSkillVersions(selected?.id, versionPage);
  const preview = useSkillPreview(selected?.id, version);
  const disabled = readOnly || write.isPending || !current.data;
  const save = (skills: AgentSkill[]) => {
    if (!current.data) return;
    write.mutate(
      { expected_revision: current.data.revision, skills },
      {
        onSuccess: () => {
          setSelected(undefined);
          setVersion("");
        },
      },
    );
  };
  const choose = (id: string, name: string) => {
    setSelected({ id, name });
    setVersionPage(1);
    setVersion("");
  };
  const replacement = current.data?.skills.find(
    (s) => s.skill_id === selected?.id,
  );
  const unchanged =
    preview.data &&
    replacement &&
    JSON.stringify(preview.data.reference) === JSON.stringify(replacement);
  const test = (skillId: string) => {
    if (testing === skillId) void tested.refetch();
    else setTesting(skillId);
  };
  const edit = (skillId: string) => {
    improve.mutate(skillId, {
      onSuccess: (result) =>
        setImproving({ skillId, proposalId: result.proposal_id }),
    });
  };
  const [activeSkill, setActiveSkill] = useState<string>();
  const [historyOpen, setHistoryOpen] = useState(false);
  useDrawerSearchEntries([
    { id: "skill-history", title: "History", breadcrumb: "Skills › History", keywords: "Skill activity", pages: ["skill-history"], reveal: () => setHistoryOpen(true) },
    ...(!readOnly ? (search.data?.items ?? []).map((item) => ({ id: `catalog-skill-${item.id}`, title: item.name, breadcrumb: "Skills › Find skills", keywords: item.description ?? "",
      pages: ["skill-version"], reveal: () => { setActiveSkill(undefined); choose(item.id, item.name); },
    })) : []),
    ...(history.data?.items ?? []).map((item) => ({ id: `skill-history-${item.id}`, title: HISTORY_LABELS[item.event_type] ?? "Skill activity",
      breadcrumb: "Skills › History", keywords: `${item.skills.map((pin) => `${pin.name} ${pin.version}`).join(" ")} ${item.created_at} Revision ${item.revision ?? ""}`,
      pages: ["skill-history"], reveal: () => setHistoryOpen(true) })),
  ]);
  const errors = [
    current.error,
    write.error,
    improve.error,
    search.error,
    versions.error,
    preview.error,
  ].filter(Boolean);
  return (
    <section
      id={id}
      tabIndex={id ? -1 : undefined}
      aria-label="Skills"
      className="space-y-6 outline-none"
    >
      <GroupedList
        title="Attached skills"
        footer={
          <>
            <p>
              Teach this agent with pinned Ornn skills. Skills never grant
              permissions. Up to 16 skills.
            </p>
            {current.data ? (
              <p>
                {current.data.skills.length}/16 skills · Revision{" "}
                {current.data.revision}
              </p>
            ) : null}
            {!activeSkill &&
              !selected &&
              errors.map((error, index) => (
                <p key={index} role="alert" className="text-destructive">
                  {error!.message}
                </p>
              ))}
          </>
        }
      >
        {current.isPending ? <GroupedRow title="Loading skills..." /> : null}
        {current.data?.skills.length === 0 ? (
          <GroupedRow title="No skills attached." />
        ) : null}
        {current.data?.skills.map((skill) => (
          <AttachedSkill
            key={skill.skill_id}
            skill={skill}
            open={activeSkill === skill.skill_id}
            onOpenChange={(open) =>
              setActiveSkill(open ? skill.skill_id : undefined)
            }
            error={
              selected
                ? undefined
                : errors.map((error) => error!.message).join(" ")
            }
            description={current.data.metadata[skill.skill_id]?.description}
            size={current.data.metadata[skill.skill_id]?.size_bytes}
            disabled={disabled || improve.isPending}
            canDisable={canDisable}
            canEdit={canEdit && !readOnly}
            test={() => test(skill.skill_id)}
            testResult={testing === skill.skill_id ? tested : undefined}
            edit={() => edit(skill.skill_id)}
            review={() => choose(skill.skill_id, skill.name)}
            toggle={(enabled) =>
              save(
                current.data!.skills.map((item) =>
                  item.skill_id === skill.skill_id
                    ? { ...item, disabled: !enabled }
                    : item,
                ),
              )
            }
            remove={() =>
              save(
                current.data.skills.filter(
                  (s) => s.skill_id !== skill.skill_id,
                ),
              )
            }
          >
            <SkillPreloadSettings
              agentId={agentId}
              skillId={skill.skill_id}
              name={skill.name}
              revision={current.data.revision}
              paths={current.data.metadata[skill.skill_id]?.preload_paths ?? []}
              bytes={current.data.metadata[skill.skill_id]?.preload_bytes ?? 0}
              disabled={
                disabled || improve.isPending || Boolean(skill.disabled)
              }
            />
            {improving?.skillId === skill.skill_id ? (
              <>
                <GroupedList
                  title="Draft"
                  footer={`Editing a copy of ${skill.name} ${skill.version}. Saving changes only the draft; the attached version stays in use until you publish the draft from its confirmation card in NyxBot's home thread.`}
                >
                  <GroupedRow title="Skill draft" />
                </GroupedList>
                <ProposalDraftEditor
                  agentId={agentId}
                  proposalId={improving.proposalId}
                />
              </>
            ) : null}
          </AttachedSkill>
        ))}
      </GroupedList>
      <GroupedList
        footer={
          history.error ? (
            <p role="alert" className="text-destructive">
              Skill history is unavailable: {history.error.message}
            </p>
          ) : undefined
        }
      >
        <GroupedDisclosure data-drawer-page-trigger="skill-history" onClick={() => setHistoryOpen(true)}>
          History
        </GroupedDisclosure>
      </GroupedList>
      <GroupedDetail
        searchPage="skill-history"
        open={historyOpen}
        title="Skill history"
        backLabel="Back to Skills"
        onBack={() => setHistoryOpen(false)}
      >
        {history.data?.items.length ? (
          <div data-drawer-search-target="skill-history"><SkillHistory items={history.data.items} /></div>
        ) : (
          <GroupedList
            data-drawer-search-target="skill-history"
            footer={
              history.error ? (
                <p role="alert" className="text-destructive">
                  Skill history is unavailable: {history.error.message}
                </p>
              ) : undefined
            }
          >
            <GroupedRow
              title={
                history.isPending
                  ? "Loading history…"
                  : history.error
                    ? "Could not load skill history."
                    : "No skill activity yet."
              }
            />
          </GroupedList>
        )}
      </GroupedDetail>
      {!readOnly ? (
        <>
          <GroupedList
            title="Find skills"
            search={{ id: "find-skills", title: "Find skills", breadcrumb: "Skills", keywords: "Search Ornn catalog" }}
            footer={
              search.data?.items.length === 0
                ? "No visible skills found."
                : undefined
            }
          >
            <GroupedRow>
              <form
                className="flex w-full items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  setQuery(input.trim());
                  setPage(1);
                }}
              >
                <GroupedInput
                  className="min-w-0 flex-1"
                  aria-label="Search Ornn skills"
                  placeholder="Search Ornn skills"
                  value={input}
                  maxLength={200}
                  onChange={(event) => setInput(event.target.value)}
                />
                <Button
                  type="submit"
                  variant="ghost"
                  size="icon"
                  aria-label="Search"
                  disabled={disabled || search.isFetching}
                >
                  <Search aria-hidden />
                </Button>
              </form>
            </GroupedRow>
            {search.data?.items.map((item) => (
              <GroupedDisclosure
                key={item.id}
                disabled={disabled}
                onClick={() => choose(item.id, item.name)}
              >
                <span className="block">{item.name}</span>
                <span className="block text-11 font-normal text-text-tertiary">
                  {item.description}
                </span>
              </GroupedDisclosure>
            ))}
            {search.data && search.data.total_pages > 1 ? (
              <GroupedRow
                title={`Page ${page}`}
                accessory={
                  <>
                    <Button
                      variant="ghost"
                      disabled={page <= 1}
                      onClick={() => setPage(page - 1)}
                    >
                      Previous
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={page >= search.data.total_pages}
                      onClick={() => setPage(page + 1)}
                    >
                      Next
                    </Button>
                  </>
                }
              />
            ) : null}
          </GroupedList>
          <GroupedDetail
            searchPage="skill-version"
            open={Boolean(selected)}
            level={activeSkill ? 2 : 1}
            title={selected?.name ?? "Choose version"}
            backLabel={activeSkill ? "Back to skill" : "Back to Skills"}
            onBack={() => {
              setSelected(undefined);
              setVersion("");
            }}
          >
            <GroupedList
              title="Choose version"
              data-drawer-search-target={selected ? `catalog-skill-${selected.id}` : undefined}
              footer={
                <>
                  {preview.isFetching ? (
                    <p role="status">Verifying package...</p>
                  ) : null}
                  {preview.data ? (
                    <>
                      <p>{preview.data.description}</p>
                      <p>
                        {preview.data.size_bytes.toLocaleString()} bytes ·{" "}
                        {preview.data.reference.dependencies.length} pinned
                        dependencies
                      </p>
                    </>
                  ) : null}
                  <p>
                    Attaching external guidance changes how this agent works.
                  </p>
                  {selected &&
                    errors.map((error, index) => (
                      <p key={index} role="alert" className="text-destructive">
                        {error!.message}
                      </p>
                    ))}
                </>
              }
            >
              <GroupedRow
                title="Version"
                accessory={
                  <Select value={version} onValueChange={setVersion}>
                    <GroupedSelectTrigger aria-label="Skill version">
                      <SelectValue placeholder="Choose an exact version" />
                    </GroupedSelectTrigger>
                    <SelectContent>
                      {versions.data?.items.map((v) => (
                        <SelectItem key={v.version} value={v.version}>
                          {v.version}
                          {v.deprecated ? " (Deprecated)" : ""}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                }
              />
              {versions.data && versions.data.total_pages > 1 ? (
                <GroupedRow
                  title={`Version page ${versionPage}`}
                  accessory={
                    <>
                      <Button
                        variant="ghost"
                        disabled={versionPage <= 1}
                        onClick={() => {
                          setVersionPage(versionPage - 1);
                          setVersion("");
                        }}
                      >
                        Newer versions
                      </Button>
                      <Button
                        variant="ghost"
                        disabled={versionPage >= versions.data.total_pages}
                        onClick={() => {
                          setVersionPage(versionPage + 1);
                          setVersion("");
                        }}
                      >
                        Older versions
                      </Button>
                    </>
                  }
                />
              ) : null}
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  disabled={
                    disabled ||
                    !preview.data ||
                    preview.isFetching ||
                    Boolean(unchanged) ||
                    (!replacement && (current.data?.skills.length ?? 16) >= 16)
                  }
                  onClick={() => {
                    if (preview.data && current.data && selected)
                      save([
                        ...current.data.skills.filter(
                          (s) => s.skill_id !== selected.id,
                        ),
                        preview.data.reference,
                      ]);
                  }}
                >
                  {replacement ? "Re-pin skill" : "Attach skill"}
                </button>
              </GroupedRow>
            </GroupedList>
          </GroupedDetail>
        </>
      ) : null}
    </section>
  );
}
