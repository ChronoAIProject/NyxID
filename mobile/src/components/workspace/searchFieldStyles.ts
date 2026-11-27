import type { TextStyle, ViewStyle } from "react-native";
import { fonts } from "../../theme/designTokens";
import { SEARCH_FIELD } from "./textFit";

/** Keep the icon, native text baseline and clear target aligned in search sheets. */
export const searchFieldContent: ViewStyle = {
  flexDirection: "row",
  alignItems: "center",
  gap: SEARCH_FIELD.gap,
  minHeight: 44,
  paddingLeft: SEARCH_FIELD.paddingLeft,
  paddingRight: 4,
};

export const searchFieldInput: TextStyle = {
  flex: 1,
  minWidth: 0,
  minHeight: 44,
  fontFamily: fonts.bodyRegular,
  fontSize: SEARCH_FIELD.fontSize,
  lineHeight: 20,
  paddingVertical: 12,
  paddingLeft: 0,
  paddingRight: SEARCH_FIELD.textPaddingRight,
  textAlignVertical: "center",
  includeFontPadding: false,
};
