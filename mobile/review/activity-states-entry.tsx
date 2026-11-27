/** Isolated review entry. Application entry points never import this file. */
import { registerRootComponent } from "expo";
import { useEffect, useState } from "react";
import { Pressable, ScrollView, Text, View } from "react-native";
import {
  useAnimatedReaction,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";
import { scheduleOnRN } from "react-native-worklets";
import {
  ROSTER_ACTIVITY_LABELS,
  ROSTER_ACTIVITY_STATES,
} from "../../shared/agent-avatars/activity";
import { defaultAvatar } from "../../shared/agent-avatars/model";
import {
  quietActivityMotion,
  sampleActivityMotion,
} from "../../shared/agent-avatars/quiet-motion";
import { AgentAvatar } from "../src/components/AgentAvatar";
import { useReducedMotion } from "../src/lib/accessibility/useReducedMotion";
import { AgentAvatarsFlag } from "../src/lib/avatars/agentAvatarsFlag";
import { ThemeProvider, useTheme } from "../src/theme/ThemeContext";

const states = [...ROSTER_ACTIVITY_STATES, "success", "failure"] as const;
const profiles = ROSTER_ACTIVITY_STATES.map((state) => ({
  state,
  signature: quietActivityMotion(state)!,
}));
const base = defaultAvatar("activity-review-agent", "specialist");
const columns = [
  { title: "List", size: "xl", animated: false },
  { title: "Home", size: "lg", animated: false },
  { title: "Chat", size: "xl", animated: true },
  { title: "Crew", size: "md", animated: true },
] as const;

function reportMotion(samples: unknown) {
  console.info("NYX_ACTIVITY_WORKLET", JSON.stringify(samples));
}

function ActivityGallery() {
  const { colors, mode, setPreference } = useTheme();
  const reduced = useReducedMotion();
  const [still, setStill] = useState(false);
  const phase = useSharedValue(0);
  useEffect(() => {
    phase.value = withTiming(1, { duration: 1100 });
  }, [phase]);
  useAnimatedReaction(
    () => phase.value >= 0.25,
    (ready, previous) => {
      if (!ready || previous) return;
      const samples = profiles.map(({ state, signature }) => ({
        state,
        ...sampleActivityMotion(signature, (signature.period ?? 0) * 0.25),
      }));
      scheduleOnRN(reportMotion, samples);
    },
  );
  useEffect(() => {
    console.info(
      "NYX_ACTIVITY_GALLERY",
      JSON.stringify({
        theme: mode,
        reduced,
        still,
        hermes: "HermesInternal" in globalThis,
      }),
    );
  }, [mode, reduced, still]);
  const foreground = { color: colors.textPrimary };
  return (
    <ScrollView
      style={{ flex: 1, backgroundColor: colors.bg }}
      contentContainerStyle={{
        paddingTop: 38,
        paddingHorizontal: 14,
        paddingBottom: 18,
      }}
    >
      <Text
        accessibilityRole="header"
        style={[foreground, { fontSize: 20, fontWeight: "600" }]}
      >
        Agent activity
      </Text>
      <View style={{ flexDirection: "row", gap: 16, paddingVertical: 12 }}>
        <Pressable
          accessibilityRole="button"
          onPress={() => setPreference(mode === "dark" ? "light" : "dark")}
        >
          <Text style={foreground}>
            {mode === "dark" ? "Dark theme" : "Light theme"}
          </Text>
        </Pressable>
        <Pressable accessibilityRole="button" onPress={() => setStill(!still)}>
          <Text style={foreground}>
            {still ? "Saved Still" : "Automatic motion"}
          </Text>
        </Pressable>
      </View>
      <Text style={foreground}>
        {reduced ? "System Reduce Motion on" : "System motion on"}
      </Text>
      <View style={{ flexDirection: "row", paddingVertical: 6 }}>
        <Text style={[foreground, { flex: 1 }]}>State</Text>
        {columns.map(({ title }) => (
          <Text
            key={title}
            style={[foreground, { width: 49, textAlign: "center" }]}
          >
            {title}
          </Text>
        ))}
      </View>
      {states.map((state) => (
        <View
          key={state}
          style={{
            minHeight: 58,
            flexDirection: "row",
            alignItems: "center",
            borderTopWidth: 1,
            borderColor: colors.borderSoft,
          }}
        >
          <Text style={[foreground, { flex: 1 }]}>
            {state === "success"
              ? "Completed"
              : state === "failure"
                ? "Failed"
                : ROSTER_ACTIVITY_LABELS[state]}
          </Text>
          {columns.map(({ title, size, animated }) => (
            <View key={title} style={{ width: 49, alignItems: "center" }}>
              <AgentAvatar
                agent={{
                  id: state,
                  name: "Review agent",
                  kind: "specialist",
                  avatar: {
                    ...base,
                    choices: {
                      ...base.choices,
                      motion: still ? "still" : "automatic",
                    },
                  },
                }}
                mood={state}
                size={size}
                animated={animated}
              />
            </View>
          ))}
        </View>
      ))}
    </ScrollView>
  );
}

registerRootComponent(() => (
  <ThemeProvider>
    <AgentAvatarsFlag value>
      <ActivityGallery />
    </AgentAvatarsFlag>
  </ThemeProvider>
));
