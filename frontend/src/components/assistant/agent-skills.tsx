import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
} from "@/hooks/use-agent-skills";
import type { AgentSkill } from "@/schemas/agent-skills";

function AttachedSkill({
  skill,
  description,
  size,
  disabled,
  remove,
  review,
}: {
  readonly skill: AgentSkill;
  readonly description?: string;
  readonly size?: number;
  readonly disabled: boolean;
  readonly remove: () => void;
  readonly review: () => void;
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
      </div>
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
  const write = useSetAgentSkills(agentId);
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
      {[current.error, write.error, search.error, versions.error, preview.error]
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
          disabled={disabled}
          review={() => choose(skill.skill_id, skill.name)}
          remove={() =>
            save(
              current.data.skills.filter((s) => s.skill_id !== skill.skill_id),
            )
          }
        />
      ))}
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
