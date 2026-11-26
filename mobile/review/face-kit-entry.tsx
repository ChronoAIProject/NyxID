/** Isolated native verification entry. Never imported by the application. */
import { registerRootComponent } from "expo";
import { useEffect, useRef, useState } from "react";
import { AppState, Pressable, ScrollView, Text, View } from "react-native";
import { File, Paths } from "expo-file-system";
import {
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import {
  AvatarAnimated,
  AvatarStill,
} from "../src/components/avatar/AvatarArt";
import { VoiceRippleBanner } from "../src/components/avatar/VoiceRippleBanner";
import { runFaceContracts } from "../../shared/agent-avatars/contract-cases";
import {
  defaultAvatar,
  AVATAR_CATALOGUE,
  type AvatarMood,
} from "../../shared/agent-avatars/model";
import {
  drawAvatar,
  maskCacheInfo,
  stillCacheInfo,
} from "../../shared/agent-avatars/draw";
import { pngAvatar, pngAvatarAsync } from "../../shared/agent-avatars/export";
import {
  prepareVoiceRippleFrames,
  voiceRippleCacheInfo,
} from "../../shared/agent-avatars/voice-ripple";
const base = defaultAvatar("nyxbot", "nyxbot"),
  surfaces = Object.keys(AVATAR_CATALOGUE.fields.surface),
  palettes = Object.keys(AVATAR_CATALOGUE.fields.ripple_palette);
function hash(value: string) {
  let h = 0x811c9dc5;
  for (let i = 0; i < value.length; i++)
    h = Math.imul(h ^ value.charCodeAt(i), 0x01000193);
  return (h >>> 0).toString(16);
}
function FaceKitReview() {
  const [report, setReport] = useState("Running native contracts…"),
    [mood, setMood] = useState<AvatarMood>("idle"),
    [palette, setPalette] = useState("nyx"),
    [still, setStill] = useState(false),
    [visible, setVisible] = useState(true),
    level = useSharedValue(0.6),
    voiceEvidence = useRef<Record<string, unknown>>({});
  const config = {
    template: "face-v1",
    choices: {
      ...base.choices,
      ripple_palette: palette,
      motion: still ? "still" : "automatic",
    },
  };
  useEffect(() => {
    level.value = withRepeat(withTiming(1, { duration: 1600 }), -1, true);
  }, [level]);
  useEffect(() => {
    const started = performance.now();
    let assertions = 0;
    void (async () => {
      try {
        runFaceContracts((ok, message) => {
          assertions++;
          if (!ok) throw Error(message);
        });
        const hashes: Record<string, string> = {};
        for (const surface of surfaces)
          hashes[surface] = hash(
            JSON.stringify(
              drawAvatar(
                { template: "face-v1", choices: { ...base.choices, surface } },
                { size: 112, native: true, kind: "nyxbot", theme: "dark" },
              ),
            ),
          );
        const exports: Record<string, number> = {};
        const exportResponsiveness: Record<string, unknown> = {};
        for (const surface of ["flat", "ascii-inverted", "dither-halftone"]) {
          const exportConfig = {
            template: "face-v1",
            choices: { ...base.choices, surface },
          };
          const syncStarted = performance.now(),
            expected = pngAvatar(exportConfig),
            syncMs = performance.now() - syncStarted;
          let heartbeats = 0,
            yields = 0,
            maxGap = 0,
            last = performance.now();
          const heartbeat = setInterval(() => {
            const now = performance.now();
            maxGap = Math.max(maxGap, now - last);
            last = now;
            heartbeats++;
          }, 16);
          const start = performance.now();
          const bytes = await pngAvatarAsync(
            exportConfig,
            false,
            "dark",
            undefined,
            {
              onProgress: () => {
                yields++;
              },
            },
          );
          maxGap = Math.max(maxGap, performance.now() - last);
          clearInterval(heartbeat);
          const equal =
            bytes.length === expected.length &&
            bytes.every((value, index) => value === expected[index]);
          if (!equal || heartbeats === 0)
            throw Error(
              `Async export parity/responsiveness failed: ${surface}`,
            );
          new File(Paths.document, `face-kit-${surface}.png`).write(bytes);
          exports[surface] = performance.now() - start;
          exportResponsiveness[surface] = {
            syncMs,
            asyncMs: exports[surface],
            heartbeats,
            yields,
            maxGap,
            byteEqual: equal,
          };
        }
        const evidence = {
          assertions,
          elapsed: performance.now() - started,
          hashes,
          exports,
          exportResponsiveness,
          maskCache: maskCacheInfo(),
          stillCache: stillCacheInfo(),
          appState: AppState.currentState,
        };
        new File(Paths.document, "face-kit-native-evidence.json").write(
          JSON.stringify(evidence, null, 2),
        );
        setReport(
          `PASS ${assertions} assertions · ${Math.round(evidence.elapsed)}ms`,
        );
      } catch (cause) {
        setReport(`FAIL ${String(cause)}`);
        new File(Paths.document, "face-kit-native-evidence.json").write(
          JSON.stringify({ error: String(cause) }),
        );
      }
    })();
  }, []);
  useEffect(() => {
    if (!report.startsWith("PASS")) return;
    const states: AvatarMood[] = [
      "thinking",
      "working",
      "drafting",
      "listening",
      "speaking",
      "approval",
      "success",
      "failure",
      "stopping",
    ];
    let index = 0;
    const timer = setInterval(() => {
      setMood(states[index % states.length]!);
      setPalette(palettes[index++ % palettes.length]!);
    }, 3000);
    return () => clearInterval(timer);
  }, [report]);
  useEffect(() => {
    if (!report.startsWith("PASS")) return;
    let live = true;
    prepareVoiceRippleFrames(palette)
      .then((frames) => {
        if (!live) return;
        voiceEvidence.current[palette] = {
          frames: frames.frames.length,
          hashes: frames.frames.map(hash),
          buildMs: frames.buildMs,
          bytes: frames.bytes,
        };
        new File(Paths.document, "face-kit-native-voice.json").write(
          JSON.stringify(
            { palettes: voiceEvidence.current, cache: voiceRippleCacheInfo() },
            null,
            2,
          ),
        );
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [palette, report]);
  if (!report.startsWith("PASS"))
    return (
      <View
        style={{
          flex: 1,
          backgroundColor: "#141416",
          padding: 24,
          paddingTop: 64,
        }}
      >
        <Text style={{ color: "#fff" }}>{report}</Text>
      </View>
    );
  return (
    <ScrollView
      onScroll={(event) => setVisible(event.nativeEvent.contentOffset.y < 500)}
      scrollEventThrottle={100}
      style={{ flex: 1, backgroundColor: "#141416" }}
      contentContainerStyle={{ padding: 24, paddingTop: 64, gap: 14 }}
    >
      <Text style={{ color: "#fff", fontSize: 22 }}>
        NyxBot face kit · native verification
      </Text>
      <Text style={{ color: "#fde047" }}>{report}</Text>
      <Text style={{ color: "#fff" }}>
        Controlled voice preview · {palette}
      </Text>
      <VoiceRippleBanner
        config={config}
        kind="nyxbot"
        name="Support"
        status={mood === "speaking" ? "Speaking preview" : "Listening preview"}
        inputAccepting={mood !== "speaking"}
        outputPlaying={mood === "speaking"}
        outputLevel={level}
        visible={visible}
      />
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
        {palettes.map((id) => (
          <Pressable
            key={id}
            accessibilityRole="button"
            onPress={() => setPalette(id)}
            style={{ padding: 7, backgroundColor: "#2a243d", borderRadius: 8 }}
          >
            <Text style={{ color: "#fff" }}>{id}</Text>
          </Pressable>
        ))}
      </View>
      <Pressable accessibilityRole="button" onPress={() => setStill(!still)}>
        <Text style={{ color: "#fff" }}>
          Motion: {still ? "Still" : "Automatic"}
        </Text>
      </Pressable>
      <View style={{ flexDirection: "row", gap: 20, alignItems: "center" }}>
        <AvatarAnimated
          config={base}
          kind="nyxbot"
          pixels={112}
          theme="dark"
          mood={mood}
          visible={visible}
        />
        <Text style={{ color: "#fff" }}>{mood}</Text>
        {[16, 24, 44].map((p) => (
          <AvatarStill
            key={p}
            config={base}
            kind="nyxbot"
            pixels={p}
            theme="dark"
          />
        ))}
      </View>
      {surfaces.map((surface) => (
        <View
          key={surface}
          style={{ flexDirection: "row", alignItems: "center", gap: 20 }}
        >
          <AvatarStill
            config={{
              template: "face-v1",
              choices: { ...base.choices, surface },
            }}
            kind="nyxbot"
            pixels={112}
            theme="dark"
          />
          <Text style={{ color: "#fff", fontSize: 16, flexShrink: 1 }}>
            {surface}
          </Text>
        </View>
      ))}
    </ScrollView>
  );
}
registerRootComponent(FaceKitReview);
