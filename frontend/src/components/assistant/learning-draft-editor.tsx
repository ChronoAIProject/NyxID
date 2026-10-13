import { useCallback, useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useFieldArray, type FieldPath } from "react-hook-form";
import { Button } from "@/components/ui/button";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useAgentSkills } from "@/hooks/use-agent-skills";
import {
  useAgentLearningReview,
  type LearningDraftBody,
} from "@/hooks/use-agent-learning-review";
import {
  skillDraftDiagnostics,
  skillDraftFieldLabel,
  skillDraftFormSchema,
  learningDraftEditable,
  type SkillDraftDiagnostic,
  type SkillDraftForm,
  type SkillPin,
  type learningProposalSchema,
} from "@/schemas/agent-skills";
import type { z } from "zod";

type Review = ReturnType<typeof useAgentLearningReview>;
type Proposal = z.infer<typeof learningProposalSchema>;
type Draft = NonNullable<Proposal["draft"]>;

const LABEL = "text-11 font-medium text-muted-foreground";

function requestFailure(cause: unknown): string {
  return cause instanceof Error && cause.message
    ? cause.message
    : "The request could not be completed. Check the draft and try again.";
}

function pinOf(pin: SkillPin): SkillPin {
  return {
    source: pin.source,
    skill_id: pin.skill_id,
    name: pin.name,
    version: pin.version,
    sha256: pin.sha256,
  };
}

function formValues(draft: Draft): SkillDraftForm {
  return {
    kind: draft.kind,
    name: draft.name,
    description: draft.description,
    skill_md: draft.skill_md,
    rationale: draft.rationale,
    safety_notes: draft.safety_notes,
    base_skill_id: draft.base_skill?.skill_id ?? "",
    files: draft.files.map((file) => ({ ...file })),
  };
}

/** Where a server diagnostic belongs in the form; null means the banner. */
function formPath(
  field: string,
  fileCount: number,
): FieldPath<SkillDraftForm> | null {
  switch (field) {
    case "name":
    case "description":
    case "skill_md":
    case "rationale":
    case "safety_notes":
    case "kind":
      return field;
    case "base_skill":
      return "base_skill_id";
    default: {
      const file = /^files\[(\d+)\]\.(path|content)$/.exec(field);
      const index = file ? Number(file[1]) : -1;
      return file && index < fileCount
        ? `files.${index}.${file[2] as "path" | "content"}`
        : null;
    }
  }
}

function fieldMessage(item: SkillDraftDiagnostic): string {
  return `${item.line ? `Line ${item.line}: ` : ""}${item.remediation}`;
}

/** The attached pins an improve draft may build on, plus its current base. */
function baseOptions(
  attached: readonly SkillPin[],
  current: SkillPin | null | undefined,
): { readonly pins: readonly SkillPin[]; readonly detached: string | null } {
  const pins = attached.map(pinOf);
  return current && !pins.some((pin) => pin.skill_id === current.skill_id)
    ? { pins: [...pins, pinOf(current)], detached: current.skill_id }
    : { pins, detached: null };
}

function BaseSkillField({
  options,
  detached,
  loadError,
  readOnly,
  form,
  onChosen,
}: {
  readonly options: readonly SkillPin[];
  readonly detached: string | null;
  readonly loadError: Error | null;
  readonly readOnly: boolean;
  readonly form: ReturnType<typeof useAppForm<SkillDraftForm>>;
  readonly onChosen: (pin: SkillPin) => void;
}) {
  return (
    <FormField
      control={form.control}
      name="base_skill_id"
      render={({ field }) => (
        <FormItem className="space-y-1">
          <FormLabel className={LABEL}>Base skill</FormLabel>
          <Select
            value={field.value}
            disabled={readOnly}
            onValueChange={(value) => {
              field.onChange(value);
              const pin = options.find((item) => item.skill_id === value);
              if (pin) onChosen(pin);
            }}
          >
            <FormControl>
              <SelectTrigger aria-label="Base skill">
                <SelectValue placeholder="Choose an attached skill" />
              </SelectTrigger>
            </FormControl>
            <SelectContent>
              {options.map((pin) => (
                <SelectItem key={pin.skill_id} value={pin.skill_id}>
                  {pin.name} · {pin.version}
                  {pin.skill_id === detached ? " (not attached)" : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {loadError ? (
            <p role="alert" className="text-11 text-destructive">
              Attached skills could not be loaded. {loadError.message}
            </p>
          ) : null}
          <FormMessage className="text-11" />
        </FormItem>
      )}
    />
  );
}

/**
 * The full draft editor for one learning proposal. Every save sends the
 * complete draft; server diagnostics render beside the field they name.
 * The parent keys it by proposal revision, so a saved draft starts fresh.
 */
export function LearningDraftEditor({
  agentId,
  proposal,
  readOnly,
  edit,
  validate,
  onSaved,
}: {
  readonly agentId: string;
  readonly proposal: Proposal & { readonly draft: Draft };
  readonly readOnly: boolean;
  readonly edit: Review["edit"];
  readonly validate: Review["validate"];
  readonly onSaved?: () => void;
}) {
  const draft = proposal.draft;
  const form = useAppForm<SkillDraftForm>({
    resolver: zodResolver(skillDraftFormSchema),
    defaultValues: formValues(draft),
  });
  // Rows are keyed by RHF's generated id, never by the editable path.
  const files = useFieldArray({ control: form.control, name: "files" });
  const [banner, setBanner] = useState<{
    readonly message: string;
    readonly items: readonly SkillDraftDiagnostic[];
  }>();
  // A passing check describes the values it checked; any edit clears it.
  const [validBytes, setValidBytes] = useState<number>();
  useEffect(() => {
    const subscription = form.watch(() => setValidBytes(undefined));
    return () => subscription.unsubscribe();
  }, [form]);
  const kind = form.watch("kind");
  // Only an improve draft needs the agent's pins; an empty id stays idle.
  const attached = useAgentSkills(kind === "improve" ? agentId : "");
  const base = baseOptions(attached.data?.skills ?? [], draft.base_skill);
  const editError =
    edit.variables?.proposalId === proposal.id ? edit.error : null;

  // Places each diagnostic on its field; the rest go to the form banner.
  const placeDiagnostics = useCallback(
    (cause: unknown) => {
      for (const item of skillDraftDiagnostics(cause)) {
        const path = formPath(item.field, form.getValues("files").length);
        if (path) {
          form.setError(path, { type: item.rule, message: fieldMessage(item) });
        }
      }
    },
    [form],
  );
  const failure = (cause: unknown) => ({
    message: requestFailure(cause),
    items: skillDraftDiagnostics(cause).filter(
      (item) => !formPath(item.field, form.getValues("files").length),
    ),
  });

  useEffect(() => {
    if (editError) placeDiagnostics(editError);
  }, [editError, placeDiagnostics]);
  const shown = banner ?? (editError ? failure(editError) : undefined);

  const body = (values: SkillDraftForm): LearningDraftBody => {
    const pin =
      values.kind === "improve"
        ? (base.pins.find((item) => item.skill_id === values.base_skill_id) ??
          null)
        : null;
    return {
      schema_version: draft.schema_version,
      kind: values.kind,
      name: values.name,
      description: values.description,
      skill_md: values.skill_md,
      files: values.files.map(({ path, content }) => ({ path, content })),
      base_skill: pin,
      rationale: values.rationale,
      safety_notes: values.safety_notes,
    };
  };

  const save = form.handleSubmit((values) => {
    setBanner(undefined);
    setValidBytes(undefined);
    const variables = {
      proposalId: proposal.id,
      revision: proposal.revision,
      draft: body(values),
    };
    if (onSaved) edit.mutate(variables, { onSuccess: onSaved });
    else edit.mutate(variables);
  });

  // Runs the proposal's own validator on the current values without saving.
  const check = form.handleSubmit(async (values) => {
    setBanner(undefined);
    setValidBytes(undefined);
    try {
      const result = await validate.mutateAsync({
        proposalId: proposal.id,
        draft: body(values),
      });
      setValidBytes(result.bytes);
    } catch (cause) {
      placeDiagnostics(cause);
      setBanner(failure(cause));
    }
  });

  const text = (
    name: "name" | "description" | "skill_md" | "rationale" | "safety_notes",
    label: string,
    multiline: boolean,
    className?: string,
  ) => (
    <FormField
      control={form.control}
      name={name}
      render={({ field }) => (
        <FormItem className="space-y-1">
          <FormLabel className={LABEL}>{label}</FormLabel>
          <FormControl>
            {multiline ? (
              <Textarea
                aria-label={label}
                className={className}
                readOnly={readOnly}
                {...field}
              />
            ) : (
              <Input aria-label={label} readOnly={readOnly} {...field} />
            )}
          </FormControl>
          <FormMessage className="text-11" />
        </FormItem>
      )}
    />
  );

  return (
    <Form {...form}>
      <form
        aria-label="Edit skill draft"
        className="space-y-3"
        onSubmit={(event) => void save(event)}
      >
        {shown ? (
          <div role="alert" className="space-y-1 text-12 text-destructive">
            <p>{shown.message}</p>
            {shown.items.map((item) => (
              <p key={`${item.field}-${item.rule}-${item.line ?? 0}`}>
                {skillDraftFieldLabel(item.field)}: {fieldMessage(item)}
              </p>
            ))}
          </div>
        ) : null}
        <p className="text-11 text-text-tertiary">
          Generated guidance is untrusted and is shown as plain text.
        </p>
        {draft.kind === "none" ? (
          <p className="text-11 text-muted-foreground">
            Kind: no skill proposed
          </p>
        ) : (
          <FormField
            control={form.control}
            name="kind"
            render={({ field }) => (
              <FormItem className="space-y-1">
                <FormLabel className={LABEL}>Skill kind</FormLabel>
                <Select
                  value={field.value}
                  disabled={readOnly}
                  onValueChange={field.onChange}
                >
                  <FormControl>
                    <SelectTrigger aria-label="Skill kind">
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value="new">New skill</SelectItem>
                    <SelectItem value="improve">
                      Improve an attached skill
                    </SelectItem>
                  </SelectContent>
                </Select>
                <FormMessage className="text-11" />
              </FormItem>
            )}
          />
        )}
        {kind === "improve" ? (
          <BaseSkillField
            options={base.pins}
            detached={base.detached}
            loadError={attached.error}
            readOnly={readOnly}
            form={form}
            onChosen={(pin) => {
              // Authored improvements must keep the base skill's name.
              if (proposal.source === "authored") form.setValue("name", pin.name);
            }}
          />
        ) : null}
        {text("name", "Skill name", false)}
        {kind === "improve" && proposal.source === "authored" ? (
          <p className="text-11 text-text-tertiary">
            An improvement keeps the base skill&apos;s name.
          </p>
        ) : null}
        {text("description", "Skill description", true)}
        {text("skill_md", "Skill draft", true, "min-h-40 font-mono text-11")}
        {text("rationale", "Rationale", true)}
        {text("safety_notes", "Safety notes", true)}
        {files.fields.map((row, index) => (
          <div
            key={row.id}
            className="space-y-2 rounded-lg border border-border/50 p-2"
          >
            <FormField
              control={form.control}
              name={`files.${index}.path`}
              render={({ field }) => (
                <FormItem className="space-y-1">
                  <FormLabel className={LABEL}>{`File ${index + 1} path`}</FormLabel>
                  <FormControl>
                    <Input
                      aria-label={`File ${index + 1} path`}
                      readOnly={readOnly}
                      {...field}
                    />
                  </FormControl>
                  <FormMessage className="text-11" />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name={`files.${index}.content`}
              render={({ field }) => (
                <FormItem className="space-y-1">
                  <FormLabel className={LABEL}>{`File ${index + 1} content`}</FormLabel>
                  <FormControl>
                    <Textarea
                      aria-label={`File ${index + 1} content`}
                      className="font-mono text-11"
                      readOnly={readOnly}
                      {...field}
                    />
                  </FormControl>
                  <FormMessage className="text-11" />
                </FormItem>
              )}
            />
            {!readOnly ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={() => files.remove(index)}
              >
                {`Remove file ${index + 1}`}
              </Button>
            ) : null}
          </div>
        ))}
        {!readOnly ? (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => files.append({ path: "", content: "" })}
            >
              Add file
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={validate.isPending}
              onClick={() => void check()}
            >
              Validate
            </Button>
            {form.formState.isDirty ? (
              <Button
                type="submit"
                size="sm"
                variant="secondary"
                disabled={edit.isPending}
              >
                Save draft
              </Button>
            ) : null}
          </div>
        ) : null}
        {validBytes !== undefined ? (
          <p role="status" className="text-11 text-success">
            The draft passes validation ({validBytes.toLocaleString("en-US")}{" "}
            bytes).
            {form.formState.isDirty ? " Save it to keep these changes." : ""}
          </p>
        ) : null}
        {proposal.source === "authored" ? (
          <p className="text-11 text-muted-foreground">
            Saving updates the draft. Authored drafts are published only from
            their confirmation card in the conversation.
          </p>
        ) : null}
      </form>
    </Form>
  );
}

/**
 * One proposal's editor, loaded by id: used where a draft is opened outside
 * the learning panel (an attached skill's Edit, an authored card).
 */
export function ProposalDraftEditor({
  agentId,
  proposalId,
  readOnly = false,
  showCardLink = true,
  onSaved,
}: {
  readonly agentId: string;
  readonly proposalId: string;
  readonly readOnly?: boolean;
  /** Off inside the card itself, which is the place to publish. */
  readonly showCardLink?: boolean;
  readonly onSaved?: () => void;
}) {
  const review = useAgentLearningReview(agentId);
  const proposal = review.data?.proposals.find((item) => item.id === proposalId);
  if (review.isPending) {
    return (
      <p role="status" className="text-11 text-muted-foreground">
        Loading the draft…
      </p>
    );
  }
  if (review.error) {
    return (
      <p role="alert" className="text-11 text-destructive">
        The draft could not be loaded. {requestFailure(review.error)}
      </p>
    );
  }
  if (!proposal?.draft) {
    return (
      <p role="status" className="text-11 text-muted-foreground">
        This draft is no longer available.
      </p>
    );
  }
  return (
    <div className="space-y-2">
      <LearningDraftEditor
        key={`${proposal.id}:${proposal.revision}`}
        agentId={agentId}
        proposal={{ ...proposal, draft: proposal.draft }}
        readOnly={readOnly || !learningDraftEditable(proposal.status)}
        edit={review.edit}
        validate={review.validate}
        onSaved={onSaved}
      />
      {showCardLink && proposal.card_conversation_id ? (
        <Link
          to="/assistant"
          search={{ c: proposal.card_conversation_id }}
          className="text-11 text-primary hover:underline"
        >
          Open the confirmation card
        </Link>
      ) : null}
    </div>
  );
}
