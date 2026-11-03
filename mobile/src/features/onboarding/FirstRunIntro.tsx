import { useEffect, useRef, useState } from "react";
import { Feather } from "@expo/vector-icons";
import {
  ActivityIndicator,
  Image,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
} from "react-native";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { SafeAreaView } from "react-native-safe-area-context";
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
import { NextStepList } from "./NextStepList";

const STEPS = ["interests", "chief", "specialists", "control", "next"] as const;
const LAST = STEPS.length - 1;

/**
 * The first-run NyxBot intro, matching the web app: a skippable interest
 * picker, three short screens, then one-tap next steps ranked by the answers.
 */
export function FirstRunIntro({
  visible,
  state,
  entryPoint,
  onClose,
  onStart,
}: {
  visible: boolean;
  state: Onboarding;
  entryPoint: "first_run" | "home";
  onClose: () => void;
  onStart: (step: NextStep) => void;
}) {
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const save = useMutation({
    mutationFn: (answers: OnboardingAnswers) => onboardingApi.save(answers),
    onSuccess: (next) => queryClient.setQueryData(onboardingKey, next),
  });
  const [index, setIndex] = useState(0);
  const [topics, setTopics] = useState<readonly string[]>([]);
  const [workStyle, setWorkStyle] = useState<string | null>(null);
  const [steps, setSteps] = useState<readonly NextStep[] | null>(null);
  const saved = useRef(false);
  const openedAt = useRef(0);
  const step = STEPS[index] ?? "interests";

  useEffect(() => {
    if (!visible) return;
    saved.current = false;
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

  function persist(outcome: OnboardingAnswers["outcome"]) {
    if (saved.current) return Promise.resolve(undefined);
    saved.current = true;
    return save
      .mutateAsync({ outcome, topics, work_style: workStyle })
      .catch(() => {
        // The server still shows the intro next time.
        saved.current = false;
        return undefined;
      });
  }

  function skip() {
    void capture({
      name: "ui.mobile_dialog_abandoned",
      props: {
        dialog_id: "nyxbot_intro",
        final_step: index,
        duration_ms: Date.now() - openedAt.current,
      },
    });
    void persist(index === LAST ? "completed" : "skipped");
    onClose();
  }

  function next() {
    if (index >= LAST) return;
    void capture({
      name: "ui.mobile_dialog_step_completed",
      props: { dialog_id: "nyxbot_intro", step: index, total_steps: STEPS.length },
    });
    const target = index + 1;
    setIndex(target);
    if (target === LAST)
      void persist("completed").then((result) =>
        setSteps(result?.recommendations ?? state.recommendations),
      );
  }

  function start(nextStep: NextStep) {
    void persist("completed");
    onClose();
    onStart(nextStep);
  }

  function chooseWorkStyle(id: string) {
    const next = workStyle === id ? null : id;
    const kept = new Set(
      topicsFor(state.topics, next ?? "solo").map((topic) => topic.id),
    );
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
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={skip}
    >
      <SafeAreaView
        style={[styles.fill, { backgroundColor: colors.bg }]}
        edges={["top", "bottom"]}
      >
        <View
          style={styles.progress}
          accessibilityLabel={`Step ${index + 1} of ${STEPS.length}`}
        >
          {STEPS.map((item, position) => (
            <View
              key={item}
              style={[
                styles.dot,
                {
                  width: position === index ? 24 : 12,
                  backgroundColor:
                    position <= index ? colors.textPrimary : colors.border,
                },
              ]}
            />
          ))}
        </View>
        <ScrollView
          key={step}
          style={styles.fill}
          contentContainerStyle={styles.body}
        >
          {step === "interests" ? (
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
              />
              <Label muted style={styles.overline}>
                WHAT SHOULD IT HELP WITH?
              </Label>
              <Chips
                choices={topicsFor(state.topics, workStyle ?? "solo")}
                selected={topics}
                onToggle={toggle}
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
              <Frame>
                <Label style={styles.strong}>NyxBot wants to send an email</Label>
                <Label muted style={styles.small}>
                  To dana@acme.com · Re: Contract redlines
                </Label>
                <View style={styles.row}>
                  <View
                    style={[
                      styles.tag,
                      {
                        backgroundColor: colors.successTone.bg,
                        borderColor: colors.successTone.border,
                      },
                    ]}
                  >
                    <Label style={[styles.small, { color: colors.successTone.text }]}>
                      Allow
                    </Label>
                  </View>
                  <View
                    style={[
                      styles.tag,
                      {
                        backgroundColor: colors.dangerTone.bg,
                        borderColor: colors.dangerTone.border,
                      },
                    ]}
                  >
                    <Label style={[styles.small, { color: colors.dangerTone.text }]}>
                      Deny
                    </Label>
                  </View>
                </View>
              </Frame>
              <Point text="Auto (the default) asks before sensitive or destructive actions." />
              <Point text="Reading never asks." />
            </>
          ) : (
            <>
              <Heading
                title="Pick your first step"
                body="One tap and NyxBot takes it from there. These stay on your home page until you're done with them."
              />
              {steps ? (
                <NextStepList steps={steps} onStart={start} />
              ) : (
                <ActivityIndicator color={colors.primary} />
              )}
            </>
          )}
        </ScrollView>
        <View style={[styles.footer, { borderColor: colors.borderSoft }]}>
          {index < LAST ? (
            <ActionButton label="Skip intro" variant="secondary" compact onPress={skip} />
          ) : null}
          <View style={styles.spacer} />
          {index > 0 && index < LAST ? (
            <ActionButton
              label="Back"
              variant="secondary"
              compact
              onPress={() => setIndex(index - 1)}
            />
          ) : null}
          <ActionButton
            label={
              index === LAST
                ? "Explore on my own"
                : index === 0
                  ? topics.length || workStyle
                    ? "Continue"
                    : "Continue without picking"
                  : "Next"
            }
            variant={index === LAST ? "secondary" : "primary"}
            compact
            onPress={index === LAST ? skip : next}
          />
        </View>
      </SafeAreaView>
    </Modal>
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

function Frame({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <View
      accessible={false}
      importantForAccessibility="no-hide-descendants"
      style={[styles.frame, { backgroundColor: colors.card, borderColor: colors.border }]}
    >
      {children}
    </View>
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
}: {
  choices: readonly OnboardingChoice[];
  selected: readonly string[];
  onToggle: (id: string) => void;
  multi?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.chips} accessibilityRole={multi ? undefined : "radiogroup"}>
      {choices.map((choice) => {
        const active = selected.includes(choice.id);
        return (
          <Pressable
            key={choice.id}
            accessibilityRole={multi ? "checkbox" : "radio"}
            accessibilityState={{ checked: active }}
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
                { backgroundColor: colors.cardSoft, borderColor: colors.border },
              ]}
            >
              <Feather
                name={featherIcon(choice.icon) as keyof typeof Feather.glyphMap}
                size={12}
                color={active ? colors.textPrimary : colors.textSecondary}
              />
            </View>
            <Label style={styles.small}>
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
  progress: {
    flexDirection: "row",
    gap: 6,
    paddingHorizontal: 20,
    paddingTop: 16,
  },
  dot: { height: 4, borderRadius: 2 },
  body: { padding: 20, gap: 16 },
  heading: { gap: 6 },
  title: { fontFamily: fonts.displaySemi, fontSize: 22, lineHeight: 28 },
  bodyText: { fontSize: 14, lineHeight: 20 },
  overline: { fontFamily: fonts.bodySemi, fontSize: 11, letterSpacing: 1.5 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  chip: {
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
  frame: { borderWidth: 1, borderRadius: 12, padding: 12, gap: 12 },
  row: { flexDirection: "row", gap: 8, alignItems: "center" },
  tag: {
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 4,
  },
  small: { fontSize: 13, lineHeight: 19 },
  strong: { fontFamily: fonts.bodySemi, fontSize: 13, lineHeight: 19 },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 20,
    paddingVertical: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  spacer: { flex: 1 },
});
