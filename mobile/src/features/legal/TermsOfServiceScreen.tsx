import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { flowStyles } from "../../theme/flowStyles";
import { mobileTheme } from "../../theme/mobileTheme";
import { spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "TermsOfService">;

type TermsSection = {
  title: string;
  paragraphs: string[];
  bullets?: string[];
};

const EFFECTIVE_DATE = "2026-02-25";

const TERMS_SECTIONS: TermsSection[] = [
  {
    title: "1. Acceptance of Terms",
    paragraphs: [
      "By creating an account or using NyxID, you agree to these Terms of Service and applicable laws.",
    ],
  },
  {
    title: "2. Account and Security",
    paragraphs: [
      "You are responsible for maintaining the confidentiality of your account credentials and for all activity under your account.",
    ],
    bullets: [
      "Use accurate registration information",
      "Enable MFA where available",
      "Promptly notify us of unauthorized access",
    ],
  },
  {
    title: "3. Permitted Use",
    paragraphs: ["NyxID may be used only for lawful identity and access management workflows."],
    bullets: [
      "Authentication and authorization operations",
      "Service connection and consent management",
      "Administrative security and audit tasks",
    ],
  },
  {
    title: "4. Prohibited Conduct",
    bullets: [
      "Attempting to bypass security controls",
      "Abusive traffic, scraping, or denial-of-service behavior",
      "Using NyxID to violate law, rights, or contracts",
      "Reverse engineering where prohibited by law",
    ],
    paragraphs: [],
  },
  {
    title: "5. Third-Party Integrations",
    paragraphs: [
      "When you connect external providers, their terms and privacy policies also apply. NyxID is not responsible for third-party service availability or policy changes.",
    ],
  },
  {
    title: "6. Availability and Changes",
    paragraphs: [
      "We may update, suspend, or discontinue features to improve security, stability, and compliance.",
    ],
  },
  {
    title: "7. Suspension or Termination",
    paragraphs: [
      "We may suspend or terminate access for violations, security risk, abuse, or legal requirements.",
    ],
  },
  {
    title: "8. Disclaimers",
    paragraphs: [
      "NyxID is provided on an \"as is\" basis to the extent permitted by law, without warranties of uninterrupted operation.",
    ],
  },
  {
    title: "9. Limitation of Liability",
    paragraphs: [
      "To the maximum extent permitted by law, NyxID is not liable for indirect, incidental, or consequential damages arising from service use.",
    ],
  },
  {
    title: "10. Privacy",
    paragraphs: [
      "Our Privacy Policy describes how data is handled and protected. By using NyxID, you also agree to that policy.",
    ],
  },
  {
    title: "11. Contact",
    paragraphs: ["Terms inquiries: legal@nyxid.com"],
  },
];

export function TermsOfServiceScreen({ navigation }: Props) {
  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra]}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="LEGAL" tone="warning" />
        <Text style={flowStyles.title}>Terms of Service</Text>
        <Text style={flowStyles.subtitle}>Effective date: {EFFECTIVE_DATE}</Text>

        <View style={flowStyles.card}>
          {TERMS_SECTIONS.map((section) => (
            <View key={section.title} style={styles.sectionWrap}>
              <Text style={styles.sectionTitle}>{section.title}</Text>
              {section.paragraphs.map((paragraph) => (
                <Text key={paragraph} style={styles.sectionBody}>
                  {paragraph}
                </Text>
              ))}
              {section.bullets?.map((bullet) => (
                <Text key={bullet} style={styles.bulletBody}>
                  • {bullet}
                </Text>
              ))}
            </View>
          ))}
        </View>

        <View style={flowStyles.actionWrap}>
          <PrimaryButton label="Back" kind="ghost" onPress={() => navigation.goBack()} />
        </View>
      </ScrollView>
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  scrollContentExtra: {
    paddingBottom: spacing.xxxl,
  },
  sectionWrap: {
    gap: spacing.xs,
  },
  sectionTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
  },
  sectionBody: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    lineHeight: 18,
  },
  bulletBody: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    lineHeight: 18,
  },
});
