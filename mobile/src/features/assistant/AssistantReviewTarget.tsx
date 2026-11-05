import { useEffect, useRef, useState, type ReactNode } from "react";
import { Text, View } from "react-native";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius } from "../../theme/designTokens";

export type RevealReviewTarget = (
  target: View,
  heading: Text,
  current: () => boolean,
) => void;

/** Selected history may arrive after the route, or settle while it is visible. */
export function AssistantReviewTarget({
  selected,
  active,
  scope,
  onReveal,
  children,
}: {
  selected: boolean;
  active: boolean;
  scope: string;
  onReveal?: RevealReviewTarget;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  const target = useRef<View>(null);
  const heading = useRef<Text>(null);
  const latest = useRef({ selected, active, scope, epoch: getAuthEpoch() });
  const callback = useRef(onReveal);
  callback.current = onReveal;
  const frame = useRef<ReturnType<typeof requestAnimationFrame> | undefined>(
    undefined,
  );
  const revealed = useRef(false);
  const revision = useRef(0);
  const [highlighted, setHighlighted] = useState(false);
  latest.current = { selected, active, scope, epoch: getAuthEpoch() };
  function reveal() {
    if (revealed.current || !selected || !active || !callback.current) return;
    if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    const request = latest.current;
    const generation = revision.current;
    frame.current = requestAnimationFrame(() => {
      frame.current = undefined;
      const current = () =>
        revision.current === generation &&
        latest.current.scope === request.scope &&
        latest.current.selected &&
        latest.current.active &&
        getAuthEpoch() === request.epoch;
      if (!current() || !target.current || !heading.current) return;
      revealed.current = true;
      setHighlighted(true);
      callback.current?.(target.current, heading.current, current);
    });
  }
  useEffect(() => {
    revealed.current = false;
    setHighlighted(false);
    reveal();
    return () => {
      revision.current++;
      if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    };
  }, [selected, active, scope]);
  useEffect(() => {
    if (!highlighted) return;
    const timer = setTimeout(() => setHighlighted(false), 4000);
    return () => clearTimeout(timer);
  }, [highlighted]);
  return (
    <View
      ref={target}
      collapsable={false}
      onLayout={reveal}
      style={
        selected
          ? {
              borderWidth: 2,
              borderColor: highlighted ? colors.primary : "transparent",
              borderRadius: radius.md,
              padding: 2,
            }
          : undefined
      }
    >
      {selected ? (
        <Text
          ref={heading}
          accessibilityRole="header"
          style={{
            color: colors.primaryOnTint,
            fontFamily: fonts.body,
            fontSize: 12,
            marginBottom: 4,
          }}
        >
          Selected confirmation
        </Text>
      ) : null}
      {children}
    </View>
  );
}
