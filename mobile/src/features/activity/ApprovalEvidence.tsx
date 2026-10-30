import { StyleSheet, Text, View } from "react-native";
import type { ChallengeDetail } from "../../lib/api/types";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

export function ApprovalEvidence({
  challenge,
}: {
  challenge: ChallengeDetail;
}) {
  const { colors } = useTheme();
  let argumentsText = challenge.tool_arguments;
  if (argumentsText) {
    try {
      argumentsText = JSON.stringify(JSON.parse(argumentsText), null, 2);
    } catch {
      /* Preserve non-JSON arguments verbatim. */
    }
  }
  const expiry = Date.parse(challenge.expires_at);
  return (
    <View style={styles.container}>
      <Text style={[styles.label, { color: colors.textSecondary }]}>
        Expires:{" "}
        {Number.isFinite(expiry)
          ? new Date(expiry).toLocaleString()
          : "Not verified"}
      </Text>
      {challenge.is_destructive === true ? (
        <Text style={[styles.label, { color: colors.danger }]}>
          This operation is destructive.
        </Text>
      ) : null}
      {argumentsText ? (
        <>
          <Text style={[styles.label, { color: colors.textSecondary }]}>
            Tool arguments
          </Text>
          <Text
            selectable
            style={[
              styles.code,
              { color: colors.textPrimary, backgroundColor: colors.cardSoft },
            ]}
          >
            {argumentsText}
          </Text>
        </>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: 8, marginTop: 12 },
  label: { fontFamily: fonts.body, fontSize: 12, lineHeight: 18 },
  code: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 20,
    padding: 12,
    borderRadius: 8,
  },
});
