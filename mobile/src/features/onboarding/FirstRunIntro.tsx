import { useEffect, useRef, useState } from "react";
import { Feather } from "@expo/vector-icons";
import {
  Image,
  Keyboard,
  LayoutAnimation,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  useWindowDimensions,
  View,
} from "react-native";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { SafeAreaView } from "react-native-safe-area-context";
import { AgentAvatar } from "../../components/AgentAvatar";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { onboardingApi } from "../../lib/api/onboardingApi";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  featherIcon,
  onboardingKey,
  topicsFor,
  type NextStep,
  type Onboarding,
  type OnboardingAnswers,
  type OnboardingChoice,
} from "./onboardingContracts";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { NextStepList } from "./NextStepList";

/** The screens after the greeting; the progress bar counts these. */
const STEPS = ["interests", "chief", "specialists", "control", "next"] as const;
type Step = "welcome" | (typeof STEPS)[number];

/**
 * Switching audience: leaving pills fade quickly, shared pills move with a
 * soft spring, new pills fade in.
 */
const PILL_LAYOUT = {
  duration: 280,
  create: {
    type: LayoutAnimation.Types.easeOut,
    property: LayoutAnimation.Properties.opacity,
    duration: 200,
    delay: 120,
  },
  update: { type: LayoutAnimation.Types.spring, springDamping: 0.9 },
  delete: {
    type: LayoutAnimation.Types.easeIn,
    property: LayoutAnimation.Properties.opacity,
    duration: 120,
  },
};

/**
 * The first-run NyxBot intro, matching the web app: a skippable interest
 * picker, three short screens, then one-tap next steps ranked by the answers.
 */
export function FirstRunIntro({
  visible,
  state,
  entryPoint,
  firstName,
  online,
  onClose,
  onStart,
}: {
  visible: boolean;
  state: Onboarding;
  entryPoint: "first_run" | "home";
  /** Greets a first-timer by name. */
  firstName?: string;
  /** Next steps need a connection; offline they stay visible but disabled. */
  online: boolean;
  onClose: () => void;
  onStart: (step: NextStep) => void;
}) {
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const queryClient = useQueryClient();
  const reducedMotion = useReducedMotion();
  const epoch = getAuthEpoch();
  const mounted = useRef(true);
  const saving = useRef(false);
  const presentation = useRef(0);
  const [saveError, setSaveError] = useState<string>();
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const save = useMutation({
    mutationFn: (answers: OnboardingAnswers) => onboardingApi.save(answers),
  });
  const [index, setIndex] = useState(0);
  const [topics, setTopics] = useState<readonly string[]>([]);
  const [workStyle, setWorkStyle] = useState<string | null>(null);
  const [steps, setSteps] = useState<readonly NextStep[] | null>(null);
  const saved = useRef<string | undefined>(undefined);
  const openedAt = useRef(0);
  // Only a first-timer is greeted; a replay starts at the questions.
  const flow: readonly Step[] =
    entryPoint === "first_run" ? ["welcome", ...STEPS] : STEPS;
  const LAST = flow.length - 1;
  const step: Step = flow[index] ?? "interests";
  const progress = (STEPS as readonly string[]).indexOf(step);

  useEffect(() => {
    presentation.current++;
    if (!visible) return;
    Keyboard.dismiss();
    saved.current = undefined;
    setSaveError(undefined);
    openedAt.current = Date.now();
    setIndex(0);
    setSteps(null);
    setTopics(state.profile?.topics ?? []);
    setWorkStyle(state.profile?.work_style ?? null);
    void capture({
      name: "ui.mobile_dialog_opened",
      props: { dialog_id: "nyxbot_intro", entry_point: entryPoint },
    });
    // Only a fresh open resets progress.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  // Saves when the answers differ from the last save, so revisiting a step
  // and changing a pick after reaching the end is kept too.
  async function persist(outcome: OnboardingAnswers["outcome"]) {
    if (saving.current || !online) return false;
    const opened = presentation.current;
    const answers = { outcome, topics, work_style: workStyle };
    const key = JSON.stringify(answers);
    if (saved.current === key) return true;
    saving.current = true;
    setSaveError(undefined);
    try {
      const result = await save.mutateAsync(answers);
      if (
        !mounted.current ||
        epoch !== getAuthEpoch() ||
        opened !== presentation.current
      )
        return false;
      queryClient.setQueryData(onboardingKey, result);
      saved.current = key;
      setSteps(result.recommendations);
      return true;
    } catch {
      if (
        mounted.current &&
        epoch === getAuthEpoch() &&
        opened === presentation.current
      )
        setSaveError(
          "Your intro could not be saved. Check your connection and try again.",
        );
      return false;
    } finally {
      saving.current = false;
    }
  }

  function dismissLocally() {
    presentation.current++;
    onClose();
  }

  async function skip() {
    const finished = index === LAST || state.profile?.outcome === "completed";
    if (!(await persist(finished ? "completed" : "skipped"))) return;
    void capture({
      name: "ui.mobile_dialog_abandoned",
      props: {
        dialog_id: "nyxbot_intro",
        final_step: index,
        duration_ms: Date.now() - openedAt.current,
      },
    });
    onClose();
  }

  function next() {
    if (index >= LAST) return;
    void capture({
      name: "ui.mobile_dialog_step_completed",
      props: {
        dialog_id: "nyxbot_intro",
        step: index,
        total_steps: flow.length,
      },
    });
    goTo(index + 1);
  }

  function goTo(target: number) {
    if (saving.current || target < 0 || target > LAST || target === index)
      return;
    setIndex(target);
    if (target === LAST) void persist("completed");
  }

  async function start(nextStep: NextStep) {
    if (!(await persist("completed"))) return;
    onClose();
    onStart(nextStep);
  }

  function chooseWorkStyle(id: string) {
    const next = workStyle === id ? null : id;
    const kept = new Set(
      topicsFor(state.topics, next ?? "solo").map((topic) => topic.id),
    );
    // Pills shared by both audiences stay; only the ones that differ animate.
    if (!reducedMotion) LayoutAnimation.configureNext(PILL_LAYOUT);
    setWorkStyle(next);
    setTopics((current) => current.filter((topic) => kept.has(topic)));
  }

  const toggle = (id: string) =>
    setTopics((current) =>
      current.includes(id)
        ? current.filter((topic) => topic !== id)
        : [...current, id],
    );

  return (
    <Modal
      visible={visible}
      animationType={reducedMotion ? "none" : "slide"}
      presentationStyle="pageSheet"
      onRequestClose={dismissLocally}
    >
      <SafeAreaView
        style={[styles.fill, { backgroundColor: colors.bg }]}
        edges={["top", "bottom", "left", "right"]}
      >
        <View style={styles.dismissRow}>
          <ActionButton
            label="Not now"
            variant="secondary"
            compact
            onPress={dismissLocally}
          />
        </View>
        {step === "welcome" ? null : (
          <View style={styles.progress} accessibilityRole="tablist">
            {STEPS.map((item, position) => (
              <Pressable
                key={item}
                accessibilityRole="tab"
                accessibilityLabel={`Step ${position + 1} of ${STEPS.length}`}
                accessibilityState={{
                  selected: position === progress,
                  disabled: save.isPending,
                }}
                disabled={save.isPending}
                hitSlop={{ top: 12, bottom: 12, left: 2, right: 2 }}
                onPress={() => goTo(flow.indexOf(item))}
                style={styles.dotTarget}
              >
                <View
                  style={[
                    styles.dot,
                    {
                      width: position === progress ? 24 : 12,
                      backgroundColor:
                        position <= progress
                          ? colors.textPrimary
                          : colors.border,
                    },
                  ]}
                />
              </Pressable>
            ))}
          </View>
        )}
        <ScrollView
          key={`${step}:${fontScale}`}
          style={styles.fill}
          contentContainerStyle={styles.body}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
        >
          {step === "welcome" ? (
            <Welcome
              firstName={firstName}
              onStart={next}
              onSkip={() => void skip()}
              disabled={!online || save.isPending}
            />
          ) : step === "interests" ? (
            <>
              <Heading
                title="Let's set NyxBot up for you"
                body="Tell us who it's for and what it should help with. We'll tailor your first suggestions and NyxBot will remember. You can skip this."
              />
              <Label muted style={styles.overline}>
                WHO IS IT FOR?
              </Label>
              <Chips
                choices={state.work_styles}
                selected={workStyle ? [workStyle] : []}
                onToggle={chooseWorkStyle}
                disabled={save.isPending}
              />
              <Label muted style={styles.overline}>
                WHAT SHOULD IT HELP WITH?
              </Label>
              <Chips
                choices={topicsFor(state.topics, workStyle ?? "solo")}
                selected={topics}
                onToggle={toggle}
                disabled={save.isPending}
                multi
              />
            </>
          ) : step === "chief" ? (
            <>
              <Heading
                title="Meet NyxBot, your chief of staff"
                body="Ask in plain words. NyxBot works across the apps you connect, keeps track of what matters and remembers how you like things done."
              />
              <Replay
                source={require("../../../assets/onboarding/chief-of-staff.gif")}
                label="Replay: the owner asks NyxBot to set up a Telegram bot; NyxBot links the setup page and keeps watching until the bot exists."
              />
              <Point text="Chat here, on the web, or in Telegram, Slack and other chat apps." />
              <Point text="It uses only the services you connect, through NyxID." />
            </>
          ) : step === "specialists" ? (
            <>
              <Heading
                title="Specialists: experts with one job"
                body="Think of specialists as people who are great at one thing. Give each a fixed role and scope, so it does that job better than anyone. Work with them directly, or let NyxBot hand them the work."
              />
              <Replay
                source={require("../../../assets/onboarding/specialists.gif")}
                label="Replay: the owner asks NyxBot for urgent issues; NyxBot hands the job to the researcher specialist, which reports back, and NyxBot answers."
              />
              <Point text="A role and scope you set: only the services and access you grant." />
              <Point text="Chat with one directly, ask NyxBot to delegate, or put several in a crew." />
            </>
          ) : step === "control" ? (
            <>
              <Heading
                title="You stay in control"
                body="NyxBot asks before it does anything sensitive. You choose how often it asks in NyxBot settings: Auto, Ask every time or Full access."
              />
              <Replay
                source={require("../../../assets/onboarding/control.gif")}
                label="Replay: the owner asks NyxBot to delete an agent key; NyxBot shows a confirmation card, the owner allows it, and NyxBot finishes the deletion."
              />
              <Point text="Auto (the default) asks before sensitive or destructive actions." />
              <Point text="Reading never asks." />
            </>
          ) : (
            <>
              <Heading
                title="Pick your first step"
                body="One tap and NyxBot takes it from there. These stay on your home page until you're done with them."
              />
              <NextStepList
                steps={steps ?? state.recommendations}
                onStart={start}
                disabled={!online || save.isPending}
              />
            </>
          )}
          {saveError || !online ? (
            <Label accessibilityRole="alert" style={{ color: colors.danger }}>
              {saveError ?? "You’re offline. Reconnect to save your intro."}
            </Label>
          ) : null}
          {step === "welcome" ? null : (
            <View style={[styles.footer, { borderColor: colors.borderSoft }]}>
              {index < LAST ? (
                <ActionButton
                  label="Skip intro"
                  variant="secondary"
                  compact
                  disabled={!online || save.isPending}
                  onPress={() => void skip()}
                />
              ) : null}
              <View style={styles.spacer} />
              {index > 0 && index < LAST && flow[index - 1] !== "welcome" ? (
                <ActionButton
                  label="Back"
                  variant="secondary"
                  compact
                  disabled={save.isPending}
                  onPress={() => goTo(index - 1)}
                />
              ) : null}
              <ActionButton
                label={
                  index === LAST
                    ? "Explore on my own"
                    : step === "interests"
                      ? topics.length || workStyle
                        ? "Continue"
                        : "Continue without picking"
                      : "Next"
                }
                variant={index === LAST ? "secondary" : "primary"}
                compact
                disabled={!online || save.isPending}
                loading={save.isPending}
                onPress={index === LAST ? () => void skip() : next}
              />
            </View>
          )}
        </ScrollView>
      </SafeAreaView>
    </Modal>
  );
}

/** The greeting a first-timer sees before any question. */
function Welcome({
  firstName,
  onStart,
  onSkip,
  disabled,
}: {
  firstName?: string;
  onStart: () => void;
  onSkip: () => void;
  disabled: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.welcome}>
      <AgentAvatar
        agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }}
        size="lg"
      />
      <View style={styles.heading}>
        <Label accessibilityRole="header" style={[styles.title, styles.center]}>
          {firstName ? `Welcome, ${firstName}` : "Welcome to NyxBot"}
        </Label>
        <Label muted style={[styles.bodyText, styles.center]}>
          I'm NyxBot, your personal chief of staff. Give me a minute and I'll
          show you how we'll work together.
        </Label>
      </View>
      <View style={styles.points}>
        {[
          ["star", "A chief of staff that works across your apps"],
          ["users", "Specialists with one job each, under your rules"],
          ["shield", "Nothing sensitive happens without your say-so"],
        ].map(([icon, text]) => (
          <View
            key={text}
            style={[
              styles.pointCard,
              { backgroundColor: colors.card, borderColor: colors.border },
            ]}
          >
            <Feather
              name={icon as keyof typeof Feather.glyphMap}
              size={14}
              color={colors.textPrimary}
            />
            <Label style={[styles.small, styles.fill]}>{text}</Label>
          </View>
        ))}
      </View>
      <ActionButton
        label="Let's get started"
        disabled={disabled}
        onPress={onStart}
      />
      <ActionButton
        label="Skip for now"
        variant="secondary"
        compact
        disabled={disabled}
        onPress={onSkip}
      />
    </View>
  );
}

function Heading({ title, body }: { title: string; body: string }) {
  return (
    <View style={styles.heading}>
      <Label accessibilityRole="header" style={styles.title}>
        {title}
      </Label>
      <Label muted style={styles.bodyText}>
        {body}
      </Label>
    </View>
  );
}

/** A recorded replay of the real assistant (fixture data); loops like a GIF. */
function Replay({ source, label }: { source: number; label: string }) {
  const { colors } = useTheme();
  return (
    <Image
      source={source}
      accessibilityLabel={label}
      resizeMode="contain"
      style={[styles.replay, { borderColor: colors.border, borderWidth: 1 }]}
    />
  );
}

function Point({ text }: { text: string }) {
  const { colors } = useTheme();
  return (
    <View style={styles.row}>
      <Label style={[styles.small, { color: colors.success }]}>✓</Label>
      <Label muted style={[styles.small, styles.fill]}>
        {text}
      </Label>
    </View>
  );
}

function Chips({
  choices,
  selected,
  onToggle,
  multi,
  disabled,
}: {
  choices: readonly OnboardingChoice[];
  selected: readonly string[];
  onToggle: (id: string) => void;
  multi?: boolean;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View
      style={styles.chips}
      accessibilityRole={multi ? undefined : "radiogroup"}
    >
      {choices.map((choice) => {
        const active = selected.includes(choice.id);
        return (
          <Pressable
            key={choice.id}
            accessibilityRole={multi ? "checkbox" : "radio"}
            accessibilityState={{ checked: active, disabled: !!disabled }}
            disabled={disabled}
            onPress={() => onToggle(choice.id)}
            style={({ pressed }) => [
              styles.chip,
              {
                borderColor: active ? colors.textPrimary : colors.controlBorder,
                backgroundColor: active ? colors.cardSoft : "transparent",
                transform: [{ scale: pressed ? 0.96 : 1 }],
              },
            ]}
          >
            <View
              style={[
                styles.chipIcon,
                {
                  backgroundColor: colors.cardSoft,
                  borderColor: colors.border,
                },
              ]}
            >
              <Feather
                name={featherIcon(choice.icon) as keyof typeof Feather.glyphMap}
                size={12}
                color={active ? colors.textPrimary : colors.textSecondary}
              />
            </View>
            <Label style={[styles.small, { flexShrink: 1 }]}>
              {choice.label}
              {active ? "  ✓" : ""}
            </Label>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  dismissRow: { alignItems: "flex-end", paddingHorizontal: 12 },
  progress: {
    flexDirection: "row",
    gap: 6,
    paddingHorizontal: 20,
    paddingTop: 16,
  },
  dot: { height: 4, borderRadius: 2 },
  dotTarget: {
    minWidth: 44,
    minHeight: 44,
    justifyContent: "center",
    alignItems: "center",
  },
  body: {
    padding: 20,
    gap: 16,
    flexGrow: 1,
    width: "100%",
    maxWidth: 680,
    alignSelf: "center",
  },
  heading: { gap: 6 },
  title: { fontFamily: fonts.displaySemi, fontSize: 22, lineHeight: 28 },
  bodyText: { fontSize: 14, lineHeight: 20 },
  overline: { fontFamily: fonts.bodySemi, fontSize: 11, letterSpacing: 1.5 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  chip: {
    maxWidth: "100%",
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    borderWidth: 1,
    borderRadius: 999,
    paddingLeft: 4,
    paddingRight: 12,
    paddingVertical: 4,
  },
  chipIcon: {
    width: 26,
    height: 26,
    borderRadius: 13,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  replay: { width: "100%", aspectRatio: 1280 / 864, borderRadius: 12 },
  row: { flexDirection: "row", gap: 8, alignItems: "center" },
  small: { fontSize: 13, lineHeight: 19 },
  footer: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
    paddingVertical: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  spacer: { flexGrow: 1 },
  welcome: { alignItems: "center", gap: 20, paddingVertical: 32 },
  center: { textAlign: "center" },
  points: { alignSelf: "stretch", gap: 8 },
  pointCard: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
});
