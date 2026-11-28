import { ChoiceGroup, ChoiceSelect } from "./agent-avatar-choices";
import { VoiceRippleBanner } from "./avatar/voice-ripple-banner";
import { drawAvatar } from "../../../../shared/agent-avatars/draw";
import { supportsNaturalEyes } from "../../../../shared/agent-avatars/emotes";
import { useAuthStore } from "@/stores/auth-store";
import {
  useEffect,
  useContext,
  useLayoutEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { ChevronLeft, ChevronRight, Dice5, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import { ChangeReview } from "@/components/ui/change-review";
import { useGroupedDraft } from "@/lib/grouped-drafts";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { useDrawerSearchEntries } from "@/lib/drawer-search";
import { AvatarStill } from "./nyxbot-agent-avatar";
import { AvatarRenderer } from "./avatar/avatar-renderer";
import { AvatarPreviewContext } from "./avatar/avatar-preview-context";
import { useAgentAvatarsEnabled } from "@/hooks/use-agent-avatars";
import { canEditAvatar } from "@/lib/assistant/agent-avatar-access";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import {
  AVATAR_MOODS,
  AVATAR_CATALOGUE,
  avatarChoiceLabel,
  avatarMoodLabel,
  defaultAvatar,
  randomAvatar,
  resolveAvatar,
  sameAvatar,
  sameStoredAvatar,
  isValidAvatar,
  chooseAvatar,
  type AvatarConfig,
  type AvatarField,
  type AvatarMood,
} from "@/lib/assistant/agent-avatar-model";
import { EDITOR_SLOTS } from "../../../../shared/agent-avatars/editor";
import { useDocumentTheme } from "@/lib/assistant/use-avatar-environment";
import { describeChanges } from "@/lib/grouped-form-changes";
import { useUpdateAgentAvatar } from "@/hooks/use-nyxbot-agents";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { cn } from "@/lib/utils";
import {
  agentAvatarFormSchema,
  type AgentAvatarForm,
} from "@/schemas/agent-avatar";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";

export function AgentAvatarEditor({
  agent,
  onEditingChange,
}: {
  agent: AssistantAgent;
  onEditingChange?: (active: boolean) => void;
}) {
  useUiTranslation();
  const enabled = useAgentAvatarsEnabled();
  const actor = useAuthStore((state) => state.user?.id);
  return enabled && canEditAvatar(agent) ? (
    <AvatarEditorPage
      key={`${actor}:${agent.id}`}
      agent={agent}
      onEditingChange={onEditingChange}
    />
  ) : null;
}
function AvatarEditorPage({
  agent,
  onEditingChange,
}: {
  agent: AssistantAgent;
  onEditingChange?: (active: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const closingDialogObserver = useRef<MutationObserver | null>(null);
  useEffect(() => () => closingDialogObserver.current?.disconnect(), []);
  const [seenAvatar, setSeenAvatar] = useState(agent.avatar);
  const [confirmedAvatar, setConfirmedAvatar] = useState<AvatarConfig | null>();
  // A confirmed save is immediately visible even before parent props refresh.
  // A subsequent server appearance replaces that local confirmation.
  if (!sameStoredAvatar(seenAvatar, agent.avatar)) {
    setSeenAvatar(agent.avatar);
    setConfirmedAvatar(undefined);
  }
  const effectiveAgent =
    confirmedAvatar === undefined
      ? agent
      : { ...agent, avatar: confirmedAvatar };
  const theme = useDocumentTheme();
  const searchPage = "avatar-editor";
  function closeEditor() {
    const focusedDialog = document.activeElement?.closest('[role="dialog"]');
    const drawer = trigger.current?.closest('[role="dialog"]');
    const restore = () => {
      // Let the discard dialog finish its own deferred focus restoration first.
      requestAnimationFrame(() =>
        setTimeout(() => {
          if (
            !trigger.current?.closest("[hidden], [inert], [aria-hidden=true]")
          )
            trigger.current?.focus({ preventScroll: true });
        }, 0),
      );
    };
    setOpen(false);
    closingDialogObserver.current?.disconnect();
    if (
      focusedDialog &&
      focusedDialog !== drawer &&
      focusedDialog.isConnected
    ) {
      const observer = new MutationObserver(() => {
        if (focusedDialog.isConnected) return;
        observer.disconnect();
        restore();
      });
      observer.observe(document.body, { childList: true, subtree: true });
      closingDialogObserver.current = observer;
    } else restore();
  }
  useDrawerSearchEntries([
    {
      id: searchPage,
      title: "Edit avatar",
      breadcrumb: translateUi("Overview") + " › " + translateUi("Avatar"),
      keywords:
        "face shape eyes palette colour background dither ASCII style reactions voice motion",
      pages: [searchPage],
      reveal: () => setOpen(true),
    },
  ]);
  return (
    <>
      <GroupedList title={translateUi("Avatar")}>
        <GroupedRow asChild interactive>
          <button
            ref={trigger}
            type="button"
            aria-label={translateUi("Edit avatar")}
            data-drawer-page-trigger={searchPage}
            onClick={() => setOpen(true)}
          >
            <AvatarStill
              config={resolveAvatar(effectiveAgent)}
              kind={agent.kind}
              theme={theme}
              pixels={112}
              className="block size-16 shrink-0"
              label={translateUi("Saved avatar")}
            />
            <span className="min-w-0 flex-1">
              <span className="block type-row-title">
                {translateUi("Edit avatar")}
              </span>
              <span className="block type-secondary text-text-tertiary">
                {translateUi("Shape, colour, style and motion")}
              </span>
            </span>
            <ChevronRight
              aria-hidden
              className="size-4 shrink-0 text-text-tertiary"
            />
          </button>
        </GroupedRow>
      </GroupedList>
      <GroupedDetail
        open={open}
        searchPage={searchPage}
        onReveal={() => setOpen(true)}
        title={translateUi("Edit avatar")}
        backLabel={translateUi("Back to Overview")}
        onBack={closeEditor}
        onActiveChange={onEditingChange}
      >
        {open ? (
          <AvatarEditorForm
            agent={effectiveAgent}
            onSaved={setConfirmedAvatar}
          />
        ) : null}
      </GroupedDetail>
    </>
  );
}
function AvatarEditorForm({
  agent,
  onSaved,
}: {
  agent: AssistantAgent;
  onSaved: (avatar: AvatarConfig | null) => void;
}) {
  const formId = useId();
  const theme = useDocumentTheme(),
    update = useUpdateAgentAvatar();
  const fallback = useMemo(
    () => defaultAvatar(agent.id, agent.kind),
    [agent.id, agent.kind],
  );
  const current = useMemo(() => resolveAvatar(agent), [agent]);
  const form = useAppForm<AgentAvatarForm>({
    resolver: zodResolver(agentAvatarFormSchema),
    defaultValues: { avatar: current },
  });
  const openingActor = useRef(useAuthStore.getState().user?.id);
  const [saved, setSaved] = useState(current);
  const baseline = useRef(agent.avatar),
    [error, setError] = useState<string>(),
    [preview, setPreview] = useState<AvatarMood>("idle");
  const [savedMessage, setSavedMessage] = useState(false);
  const saving = useRef(false),
    [busy, setBusy] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const [adopted, setAdopted] = useState(false),
    [shuffling, setShuffling] = useState(false);
  const [shuffleEvent, setShuffleEvent] = useState(0);
  const [previewEvent, setPreviewEvent] = useState(0);
  const avatar = form.watch("avatar"),
    unsupported = !!agent.avatar && !isValidAvatar(agent.avatar) && !adopted;
  const dirty = form.formState.isDirty || adopted;
  function choose(next: AvatarConfig) {
    if (saving.current) return;
    setError(undefined);
    setSavedMessage(false);
    setShuffling(false);
    form.setValue("avatar", next);
  }
  async function save(values: AgentAvatarForm) {
    if (unsupported || saving.current) return;
    saving.current = true;
    setBusy(true);
    setError(undefined);
    try {
      const latest = (await nyxBotApi.agent(agent.id)).agent;
      if (!mounted.current) return;
      if (useAuthStore.getState().user?.id !== openingActor.current)
        throw new Error(
          "Your account changed. Reopen this avatar before saving.",
        );
      if (!canEditAvatar(latest))
        throw new Error("This agent's avatar can no longer be changed here.");
      if (!sameStoredAvatar(latest.avatar, baseline.current))
        throw new Error(
          "This avatar changed elsewhere. Go back and review it before saving.",
        );
      const clearing = sameAvatar(values.avatar, fallback);
      await update.mutateAsync({
        id: agent.id,
        avatar: clearing ? null : values.avatar,
      });
      if (!mounted.current) return;
      baseline.current = clearing ? null : values.avatar;
      setSaved(clearing ? fallback : values.avatar);
      form.reset(values);
      setAdopted(false);
      setSavedMessage(true);
      onSaved(clearing ? null : values.avatar);
    } catch (cause) {
      if (!mounted.current) return;
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save the avatar. Try again.",
      );
    } finally {
      saving.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const slots = EDITOR_SLOTS;
  const [step, setStep] = useState(0);
  const stepButtons = useRef<(HTMLButtonElement | null)[]>([]);
  const stepNavigation = useRef(false);
  function goToStep(next: number) {
    if (next === step) return;
    stepNavigation.current = true;
    setStep(next);
    setShuffling(false);
    setPreview(next === 2 ? "listening" : "idle");
    setPreviewEvent((value) => value + 1);
  }
  useLayoutEffect(() => {
    if (!stepNavigation.current) return;
    stepNavigation.current = false;
    stepButtons.current[step]?.focus({ preventScroll: true });
    stepButtons.current[step]?.scrollIntoView?.({ block: "start" });
  }, [step]);
  const field = (key: AvatarField, label: string) => (
    <ChoiceGroup
      field={key}
      label={translateUi(label)}
      avatar={avatar}
      kind={agent.kind}
      theme={theme}
      onChoose={(id) => choose(chooseAvatar(avatar, key, id))}
    />
  );
  const entry = {
    label: translateUi("Avatar"),
    dirty: dirty && !unsupported,
    pending: busy || update.isPending,
    save: form.handleSubmit(save),
    changes: () =>
      describeChanges(
        saved.choices,
        form.getValues().avatar.choices,
        slots.map((slot) => ({
          key: slot.field,
          label: slot.label,
          format: (value: unknown) =>
            slot.field.startsWith("react_") &&
            value ===
              AVATAR_CATALOGUE.default_reactions[
                slot.field.slice(6) as keyof typeof AVATAR_CATALOGUE.default_reactions
              ]
              ? "Natural"
              : avatarChoiceLabel(slot.field, String(value ?? "")),
        })),
      ),
    discard: () => {
      form.reset({ avatar: saved });
      setAdopted(false);
      setError(undefined);
      setSavedMessage(false);
      setShuffling(false);
    },
  };
  useGroupedDraft(entry);
  const publishPreview = useContext(AvatarPreviewContext);
  const previewMood = shuffling ? "choosing" : step === 2 ? preview : "idle";
  const previewKey = shuffling
    ? `shuffle:${shuffleEvent}`
    : `preview:${previewEvent}`;
  useEffect(() => {
    publishPreview?.({
      agentId: agent.id,
      config: avatar,
      mood: previewMood,
      eventKey: previewKey,
    });
    return () => publishPreview?.(null);
  }, [publishPreview, agent.id, avatar, previewMood, previewKey]);
  const stepTitles = [
    "Choose a face",
    "Choose colours and style",
    "Choose voice and motion",
  ];
  const stepHelp = [
    "Pick a head shape and resting eyes.",
    "Choose a colour palette, texture and background.",
    "Set how your avatar reacts and how voice looks.",
  ];
  return (
    <section aria-label={translateUi("Avatar")} className="space-y-3">
      <Form {...form}>
        <ChangeReview
          entries={[entry]}
          save={entry.save}
          disabled={busy || unsupported}
        >
          {(review) => (
            <form
              noValidate
              aria-label={translateUi("Avatar")}
              onSubmit={(event) => {
                event.preventDefault();
                if (entry.dirty && !entry.pending && !unsupported) review();
              }}
            >
              <div className="sticky top-0 z-10 mb-4 flex flex-wrap items-center justify-end gap-x-3 gap-y-2 border-b border-hairline bg-background py-3">
                <p
                  className="mr-auto type-secondary text-text-tertiary"
                  role="status"
                >
                  {busy
                    ? translateUi("Saving…")
                    : dirty
                      ? translateUi("Unsaved changes")
                      : savedMessage
                        ? translateUi("Avatar saved.")
                        : null}
                </p>
                {entry.dirty ? (
                  <Button
                    type="button"
                    variant="ghost"
                    aria-label={translateUi("Discard avatar")}
                    disabled={entry.pending}
                    onClick={entry.discard}
                  >
                    {translateUi("Discard")}
                  </Button>
                ) : null}
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!entry.dirty || entry.pending || unsupported}
                >
                  {translateUi("Save avatar")}
                </Button>
                <FormSubmitErrors className="basis-full" />
                {error ? (
                  <p
                    className="basis-full type-secondary text-destructive"
                    role="alert"
                  >
                    {translateUi(error)}
                  </p>
                ) : null}
              </div>
              <fieldset disabled={busy} className="contents">
                <GroupedList
                  info={translateUi(
                    "Preview states do not change live activity.",
                  )}
                  search={{
                    id: "avatar-choices",
                    title: "Avatar",
                    breadcrumb: "Overview",
                    keywords:
                      "face eyes palette background dither ASCII reactions",
                  }}
                  footer={translateUi(
                    "Changes apply everywhere this agent appears.",
                  )}
                >
                  <GroupedRow className="flex-col items-stretch gap-3 py-3">
                    <nav aria-label={translateUi("Avatar steps")}>
                      <ol className="relative grid grid-cols-3 gap-2">
                        <li
                          aria-hidden="true"
                          className="absolute left-[16.67%] right-[16.67%] top-4 border-t border-hairline"
                        />
                        {["Face", "Style", "Motion"].map((name, index) => (
                          <li key={name} className="relative">
                            <button
                              ref={(button) => {
                                stepButtons.current[index] = button;
                              }}
                              type="button"
                              aria-current={index === step ? "step" : undefined}
                              aria-controls={
                                unsupported ? undefined : `${formId}-step`
                              }
                              disabled={unsupported}
                              onClick={() => goToStep(index)}
                              className={cn(
                                "flex w-full flex-col items-center gap-1.5 rounded-lg py-1 type-row-title text-text-tertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-focus",
                                step === index && "text-foreground",
                              )}
                            >
                              <span
                                aria-hidden="true"
                                className={cn(
                                  "flex size-8 items-center justify-center rounded-full border border-hairline bg-card type-row-title",
                                  step === index &&
                                    "border-nyx-500 bg-nyx-500 text-white",
                                )}
                              >
                                {index + 1}
                              </span>
                              {translateUi(name)}
                            </button>
                          </li>
                        ))}
                      </ol>
                    </nav>
                    <div className="space-y-1">
                      <p className="type-secondary text-text-tertiary">
                        {translateUi("Step {{current}} of {{total}}", {
                          current: step + 1,
                          total: 3,
                        })}
                      </p>
                      <h4 className="type-card-title">
                        {translateUi(stepTitles[step]!)}
                      </h4>
                      <p className="type-secondary">
                        {translateUi(stepHelp[step]!)}
                      </p>
                    </div>
                    {unsupported ? (
                      <div className="space-y-2">
                        <p className="type-secondary">
                          {translateUi(
                            "This appearance uses a different version. Adopt the face kit or reset explicitly to change it.",
                          )}
                        </p>
                        <Button
                          type="button"
                          onClick={() => {
                            setAdopted(true);
                            choose(current);
                          }}
                        >
                          {translateUi("Adopt face kit")}
                        </Button>
                      </div>
                    ) : null}
                    <div className="flex items-center justify-between gap-4">
                      {!publishPreview ? (
                        <div className="shrink-0 space-y-1.5">
                          <AvatarRenderer
                            kind={agent.kind}
                            config={avatar}
                            theme={theme}
                            mood={
                              shuffling
                                ? "choosing"
                                : step === 2
                                  ? preview
                                  : "idle"
                            }
                            eventKey={
                              shuffling
                                ? `shuffle:${shuffleEvent}`
                                : `preview:${previewEvent}`
                            }
                            pixels={96}
                            className="block size-24"
                            label={translateUi("Avatar preview")}
                          />
                        </div>
                      ) : null}
                      <div className="flex min-w-0 flex-col items-end gap-3">
                        <div className="flex flex-wrap justify-end gap-1">
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            disabled={unsupported}
                            onClick={() => {
                              choose(randomAvatar(Math.random, agent.kind));
                              setShuffling(true);
                              setShuffleEvent((value) => value + 1);
                            }}
                          >
                            <Dice5 aria-hidden className="size-4" />
                            {translateUi("Randomise")}
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            aria-label={translateUi("Reset to default")}
                            disabled={
                              !unsupported && sameAvatar(avatar, fallback)
                            }
                            onClick={() => {
                              setAdopted(true);
                              choose(fallback);
                            }}
                          >
                            <RotateCcw aria-hidden className="size-4" />
                            {translateUi("Reset")}
                          </Button>
                        </div>
                      </div>
                    </div>
                    {drawAvatar(avatar, {
                      size: 96,
                      mood: step === 2 ? preview : "idle",
                      kind: agent.kind,
                    }).fallback === "eye-grid" ? (
                      <p className="type-secondary">
                        {translateUi(
                          "A crisp face preserves these eyes at this texture's grid size.",
                        )}
                      </p>
                    ) : null}
                  </GroupedRow>
                  {!unsupported ? (
                    <GroupedRow
                      id={`${formId}-step`}
                      className="flex-col items-stretch gap-4 py-4"
                      aria-label={translateUi(
                        ["Face", "Style", "Motion"][step]!,
                      )}
                    >
                      {step === 0 ? (
                        <>
                          {field("head", "Head")}
                          {field("eyes", "Eyes")}
                        </>
                      ) : null}
                      {step === 1 ? (
                        <>
                          {field("palette", "Colour")}
                          {field("surface", "Texture")}
                          {field("background", "Background")}
                          <details className="rounded-lg border border-hairline p-3">
                            <summary className="cursor-pointer type-row-title focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-focus">
                              {translateUi("More appearance options")}
                            </summary>
                            <div className="mt-3 space-y-4">
                              {field("treatment", "Finish")}
                              {field("frame", "Frame")}
                            </div>
                          </details>
                        </>
                      ) : null}
                      {step === 2 ? (
                        <>
                          {field("motion", "Motion")}
                          {!supportsNaturalEyes(avatar.choices.surface ?? "flat") ? (
                            <p className="type-secondary">
                              {translateUi("This texture keeps the eyes still to preserve its pattern. Voice ripples remain available.")}
                            </p>
                          ) : null}
                          <ChoiceSelect
                            label={translateUi("Preview expression")}
                            value={preview}
                            values={AVATAR_MOODS}
                            optionLabel={(value) =>
                              translateUi(avatarMoodLabel(value as AvatarMood))
                            }
                            onChange={(value) => {
                              setPreview(value as AvatarMood);
                              setPreviewEvent((event) => event + 1);
                            }}
                          />
                          {field("ripple_palette", "Voice colour")}
                          <VoiceRippleBanner
                            config={avatar}
                            kind={agent.kind}
                            name={agent.display_name?.trim() || agent.name}
                            status={translateUi("Voice preview")}
                            inputAccepting={preview === "listening"}
                            outputPlaying={preview === "speaking"}
                            outputLevel={0.6}
                          />
                          <details className="rounded-lg border border-hairline p-3">
                            <summary className="cursor-pointer type-row-title focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-focus">
                              {translateUi("Custom reactions")}
                            </summary>
                            <p className="mt-2 type-secondary">
                              {translateUi(
                                "Choose how your avatar responds to each activity.",
                              )}
                            </p>
                            <div className="mt-3 space-y-3">
                              {slots
                                .filter((slot) =>
                                  slot.field.startsWith("react_"),
                                )
                                .map((slot) => (
                                  <div key={slot.field}>
                                    {field(slot.field, slot.label)}
                                  </div>
                                ))}
                            </div>
                          </details>
                        </>
                      ) : null}
                      <div className="flex items-center justify-between gap-3 border-t border-hairline pt-3">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          disabled={step === 0}
                          onClick={() => goToStep(step - 1)}
                        >
                          <ChevronLeft aria-hidden className="size-4" />
                          {translateUi("Back")}
                        </Button>
                        {step < 2 ? (
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={() => goToStep(step + 1)}
                          >
                            {translateUi(
                              step === 0 ? "Next: Style" : "Next: Motion",
                            )}
                            <ChevronRight aria-hidden className="size-4" />
                          </Button>
                        ) : null}
                      </div>
                    </GroupedRow>
                  ) : null}
                </GroupedList>
              </fieldset>
            </form>
          )}
        </ChangeReview>
      </Form>
    </section>
  );
}
