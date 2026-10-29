import { Text, useWindowDimensions, type TextProps } from "react-native";

/** Refresh native paragraph metrics after a live font change, without replacing its control or screen. */
export function DynamicTypeText(props: TextProps) {
  const { fontScale } = useWindowDimensions();
  return <Text key={fontScale} {...props} />;
}
