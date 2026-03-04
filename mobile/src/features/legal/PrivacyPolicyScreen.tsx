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

type Props = NativeStackScreenProps<RootStackParamList, "PrivacyPolicy">;

type LegalSection = {
  title: string;
  paragraphs: string[];
  bullets?: string[];
};

const EFFECTIVE_DATE = "2026-02-25";

const PRIVACY_SECTIONS: LegalSection[] = [
  {
    title: "1. Introduction",
    paragraphs: [
      "NyxID is an identity and access management platform. This policy explains how we collect, use, store, and protect your data.",
      "By using NyxID, you agree to this Privacy Policy.",
    ],
  },
  {
    title: "2. Information We Collect",
    bullets: [
      "Account information: email, display name, password hash",
      "Authentication data: session tokens, refresh tokens, MFA secrets",
      "Technical data: IP, user-agent, device metadata, security timestamps",
    ],
    paragraphs: [],
  },
  {
    title: "3. How We Use Your Information",
    bullets: [
      "Authenticate identity and manage sessions",
      "Enforce security policies and monitor abuse",
      "Support OAuth/SSO and connected providers",
      "Send verification and password reset notifications",
      "Maintain audit and compliance logs",
    ],
    paragraphs: [],
  },
  {
    title: "4. Data Storage and Security",
    paragraphs: [
      "Sensitive fields are encrypted at rest. Passwords are stored as secure hashes and never in plaintext.",
      "Transport security uses TLS. Access tokens and sessions are managed with scoped expiry and revocation controls.",
    ],
  },
  {
    title: "5. Data Sharing",
    paragraphs: ["NyxID does not sell personal data."],
    bullets: [
      "With your consent for connected third-party services",
      "When required by law or legal process",
      "To protect security, prevent fraud, or abuse",
    ],
  },
  {
    title: "6. Data Retention",
    paragraphs: [
      "Account data is retained while your account is active.",
      "After account deletion, personal data is removed per retention policy, with security logs retained only as needed for compliance.",
    ],
  },
  {
    title: "7. Your Rights",
    bullets: [
      "Access and export your data",
      "Correct profile information",
      "Delete account and revoke sessions",
      "Disconnect third-party integrations",
    ],
    paragraphs: [],
  },
  {
    title: "8. Cookies and Local Storage",
    paragraphs: [
      "NyxID uses secure session mechanisms and local app storage for authentication continuity and security workflows.",
    ],
  },
  {
    title: "9. Children's Privacy",
    paragraphs: [
      "NyxID is not intended for children under applicable legal age. If you believe data was collected in error, contact support for removal.",
    ],
  },
  {
    title: "10. Policy Updates",
    paragraphs: [
      "We may revise this policy over time. Material updates are reflected by a new effective date.",
    ],
  },
  {
    title: "11. Contact",
    paragraphs: ["Privacy inquiries: privacy@nyxid.com"],
  },
];

export function PrivacyPolicyScreen({ navigation }: Props) {
  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra]}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="LEGAL" tone="info" />
        <Text style={flowStyles.title}>Privacy Policy</Text>
        <Text style={flowStyles.subtitle}>Effective date: {EFFECTIVE_DATE}</Text>

        <View style={flowStyles.card}>
          {PRIVACY_SECTIONS.map((section) => (
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
