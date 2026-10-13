import { useState, type ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";
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

const HISTORY_LABELS: Record<string, string> = {
  assistant_skills_updated: "Attached skills changed",
  assistant_learning_proposal_edited: "Skill draft edited",
  assistant_learning_proposal_rejected: "Skill draft rejected",
  assistant_learning_proposal_reprepared: "Skill draft package rebuilt",
  assistant_learning_publication_approved: "Skill publication approved",
  assistant_learning_publication_deferred: "Skill publication deferred",
  assistant_learning_skill_pinned: "Published skill attached",
};

function SkillHistory({ items }: { readonly items: readonly AgentSkillHistoryItem[] }) {
  return (
    <div className="space-y-1">
      <p className="text-12 font-medium">Skill history</p>
      <ol aria-label="Skill history" className="space-y-1">
        {items.map((item) => (
          <li key={item.id} className="text-11 text-muted-foreground">
            <span className="font-medium text-foreground">
              {HISTORY_LABELS[item.event_type] ?? "Skill activity"}
            </span>{" "}
            · {new Date(item.created_at).toLocaleString()}
            {item.revision !== null ? ` · Revision ${item.revision}` : ""}
            {item.skills.length > 0 ? (
              <span className="block">
                {item.skills
                  .map(
                    (pin) =>
                      `${pin.name} ${pin.version}${pin.disabled ? " (disabled)" : ""}`,
                  )
                  .join(", ")}
              </span>
            ) : null}
          </li>
        ))}
      </ol>
    </div>
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
      <p role="status" className="text-11 text-muted-foreground">
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
      <p className="text-11 text-success">
        Test passed: the agent reads {data.name} {data.version} (
        {data.size_bytes.toLocaleString()} bytes).
      </p>
      {data.excerpt ? (
        <pre
          aria-label={`${data.name} SKILL.md excerpt`}
          className="max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-2 text-11"
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
}) {
  const versions = useSkillVersions(skill.skill_id);
  const latest = versions.data?.items[0]?.version;
  return (
    <div className="space-y-2 rounded-lg border border-border/50 p-3">
      <p className="text-12 font-medium">
        {skill.name}{" "}
        <span className="font-mono text-text-tertiary">{skill.version}</span>
      </p>
      <p className="text-12 text-muted-foreground">{description}</p>
      {size !== undefined && (
        <p className="text-11 text-text-tertiary">
          {size.toLocaleString()} bytes
        </p>
      )}
      {versions.error && (
        <p className="text-11 text-text-tertiary">
          Could not check for updates. Your pinned version is unchanged.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={disabled} onClick={review}>
          {latest && latest !== skill.version
            ? `Newer version available: ${latest}`
            : "Choose version"}
        </Button>
        <Button size="sm" disabled={disabled} onClick={remove}>
          Remove {skill.name}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          aria-label={`Test ${skill.name}`}
          disabled={testResult?.isFetching}
          onClick={test}
        >
          Test
        </Button>
        {canEdit ? (
          <Button
            size="sm"
            variant="secondary"
            aria-label={`Edit ${skill.name}`}
            disabled={disabled}
            onClick={edit}
          >
            Edit
          </Button>
        ) : null}
        {canDisable ? (
          <label className="flex items-center gap-2 text-11">
            <Switch
              checked={!skill.disabled}
              disabled={disabled}
              aria-label={`Use ${skill.name}`}
              onCheckedChange={toggle}
            />
            {skill.disabled ? "Disabled" : "Enabled"}
          </label>
        ) : skill.disabled ? (
          <span className="text-11 text-text-tertiary">Disabled</span>
        ) : null}
      </div>
      {testResult ? <TestResult result={testResult} /> : null}
      {children}
    </div>
  );
}

export function AgentSkills({
  agentId,
  readOnly = false,
}: {
  readonly agentId: string;
  readonly readOnly?: boolean;
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
  return (
    <section aria-label="Skills" className="space-y-3">
      <div className="space-y-1">
        <h3 className="text-13 font-semibold">Skills</h3>
        <p className="text-12 text-muted-foreground">
          Teach this agent with pinned Ornn skills. Skills never grant
          permissions. Up to 16 skills.
        </p>
      </div>
      {current.data && (
        <p className="text-11 text-text-tertiary">
          {current.data.skills.length}/16 skills · Revision{" "}
          {current.data.revision}
        </p>
      )}
      {current.isPending && (
        <p className="text-12 text-muted-foreground">Loading skills...</p>
      )}
      {[
        current.error,
        write.error,
        improve.error,
        search.error,
        versions.error,
        preview.error,
      ]
        .filter(Boolean)
        .map((e, i) => (
          <ErrorBanner key={i} message={e!.message} />
        ))}
      {current.data?.skills.length === 0 && (
        <p className="text-12 text-muted-foreground">No skills attached.</p>
      )}
      {current.data?.skills.map((skill) => (
        <AttachedSkill
          key={skill.skill_id}
          skill={skill}
          description={current.data.metadata[skill.skill_id]?.description}
          size={current.data.metadata[skill.skill_id]?.size_bytes}
          disabled={disabled || improve.isPending}
          canDisable={canDisable}
          canEdit={canEdit && !readOnly}
          test={() => test(skill.skill_id)}
          testResult={testing === skill.skill_id ? tested : undefined}
          edit={() => edit(skill.skill_id)}
          review={() => choose(skill.skill_id, skill.name)}
          toggle={(enabled) => save(current.data!.skills.map((item) => item.skill_id === skill.skill_id ? { ...item, disabled: !enabled } : item))}
          remove={() =>
            save(
              current.data.skills.filter((s) => s.skill_id !== skill.skill_id),
            )
          }
        >
          {improving?.skillId === skill.skill_id ? (
            <div className="space-y-2 border-t border-border/50 pt-2">
              <p className="text-11 text-muted-foreground">
                Editing a copy of {skill.name} {skill.version}. Saving changes
                only the draft; the attached version stays in use until you
                publish the draft from its confirmation card in NyxBot&apos;s
                home thread.
              </p>
              <ProposalDraftEditor
                agentId={agentId}
                proposalId={improving.proposalId}
              />
            </div>
          ) : null}
        </AttachedSkill>
      ))}
      {history.data?.items.length ? (
        <SkillHistory items={history.data.items} />
      ) : null}
      {history.error ? (
        <p role="alert" className="text-11 text-destructive">
          Skill history is unavailable: {history.error.message}
        </p>
      ) : null}
      {!readOnly && (
        <>
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              setQuery(input.trim());
              setPage(1);
            }}
          >
            <Input
              aria-label="Search Ornn skills"
              placeholder="Search Ornn skills"
              value={input}
              maxLength={200}
              onChange={(e) => setInput(e.target.value)}
            />
            <Button
              type="submit"
              disabled={disabled}
              isLoading={search.isFetching}
            >
              Search
            </Button>
          </form>
          {search.data?.items.map((item) => (
            <div
              key={item.id}
              className="space-y-1 rounded-lg border border-border/50 p-3"
            >
              <Button
                variant="ghost"
                disabled={disabled}
                onClick={() => choose(item.id, item.name)}
              >
                {item.name}
              </Button>
              <p className="text-12 text-muted-foreground">
                {item.description}
              </p>
            </div>
          ))}
          {search.data?.items.length === 0 && (
            <p className="text-12 text-muted-foreground">
              No visible skills found.
            </p>
          )}
          {search.data && search.data.total_pages > 1 && (
            <div className="flex items-center gap-2">
              <Button disabled={page <= 1} onClick={() => setPage(page - 1)}>
                Previous
              </Button>
              <span className="text-11">Page {page}</span>
              <Button
                disabled={page >= search.data.total_pages}
                onClick={() => setPage(page + 1)}
              >
                Next
              </Button>
            </div>
          )}
          {selected && (
            <div className="space-y-3 rounded-lg border border-border/50 p-3">
              <p className="text-12 font-medium">{selected.name}</p>
              <Select value={version} onValueChange={setVersion}>
                <SelectTrigger aria-label="Skill version">
                  <SelectValue placeholder="Choose an exact version" />
                </SelectTrigger>
                <SelectContent>
                  {versions.data?.items.map((v) => (
                    <SelectItem key={v.version} value={v.version}>
                      {v.version}
                      {v.deprecated ? " (Deprecated)" : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {versions.data && versions.data.total_pages > 1 && (
                <div className="flex items-center gap-2">
                  <Button
                    disabled={versionPage <= 1}
                    onClick={() => {
                      setVersionPage(versionPage - 1);
                      setVersion("");
                    }}
                  >
                    Newer versions
                  </Button>
                  <span className="text-11">
                    Version page {versionPage}
                  </span>
                  <Button
                    disabled={versionPage >= versions.data.total_pages}
                    onClick={() => {
                      setVersionPage(versionPage + 1);
                      setVersion("");
                    }}
                  >
                    Older versions
                  </Button>
                </div>
              )}
              {preview.isFetching && (
                <p className="text-12 text-muted-foreground">
                  Verifying package...
                </p>
              )}
              {preview.data && (
                <>
                  <p className="text-12 text-muted-foreground">
                    {preview.data.description}
                  </p>
                  <p className="text-11 text-text-tertiary">
                    {preview.data.size_bytes.toLocaleString()} bytes ·{" "}
                    {preview.data.reference.dependencies.length} pinned
                    dependencies
                  </p>
                </>
              )}
              <p className="text-11 text-text-tertiary">
                Attaching external guidance changes how this agent works.
              </p>
              <Button
                variant="primary"
                isLoading={write.isPending}
                disabled={
                  disabled ||
                  !preview.data ||
                  preview.isFetching ||
                  Boolean(unchanged) ||
                  (!replacement && (current.data?.skills.length ?? 16) >= 16)
                }
                onClick={() => {
                  if (preview.data && current.data)
                    save([
                      ...current.data.skills.filter(
                        (s) => s.skill_id !== selected.id,
                      ),
                      preview.data.reference,
                    ]);
                }}
              >
                {replacement ? "Re-pin skill" : "Attach skill"}
              </Button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
