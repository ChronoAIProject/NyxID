import { useState, type ComponentPropsWithRef, type ReactNode } from "react";
import { TextInput, View, type StyleProp, type ViewStyle } from "react-native";
import { useAuthTheme } from "./useAuthTheme";
import { authInputChrome } from "./authAppearance";

/** A single quiet focus edge for standalone fields and icon-adorned sign-in rows. */
export function AuthTextInput({
  leading,
  trailing,
  frameStyle,
  invalid = false,
  onFocus,
  onBlur,
  style,
  ...props
}: ComponentPropsWithRef<typeof TextInput> & {
  leading?: ReactNode;
  trailing?: ReactNode;
  frameStyle?: StyleProp<ViewStyle>;
  invalid?: boolean;
}) {
  const { colors, mode } = useAuthTheme();
  const [focused, setFocused] = useState(false);
  const chrome = authInputChrome(mode, focused, invalid);
  const input = (
    <TextInput
      placeholderTextColor={mode === "light" ? colors.textMuted : undefined}
      selectionColor={mode === "light" ? colors.primary : undefined}
      {...props}
      style={frameStyle ? style : [style, chrome]}
      onFocus={(event) => {
        setFocused(true);
        onFocus?.(event);
      }}
      onBlur={(event) => {
        setFocused(false);
        onBlur?.(event);
      }}
    />
  );
  return frameStyle ? (
    <View style={[frameStyle, chrome]}>
      {leading}
      {input}
      {trailing}
    </View>
  ) : (
    input
  );
}
