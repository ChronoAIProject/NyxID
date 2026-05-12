import { BlurView } from "expo-blur";
import { Pressable, StyleSheet, Text } from "react-native";

type Props = {
  onPress: () => void;
};

export function BlurBackButton({ onPress }: Props) {
  return (
    <Pressable onPress={onPress} style={styles.wrapper} hitSlop={8}>
      <BlurView intensity={40} tint="dark" style={styles.blur}>
        <Text style={styles.arrow}>{"\u2190"}</Text>
      </BlurView>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    alignSelf: "flex-start",
    borderRadius: 20,
    overflow: "hidden",
    marginBottom: 12,
  },
  blur: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(255,255,255,0.08)",
  },
  arrow: {
    fontSize: 18,
    color: "#e8e4f0",
    fontWeight: "600",
    marginTop: -1,
  },
});
