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

const EFFECTIVE_DATE = "2026-03-11";

const TERMS_SECTIONS: TermsSection[] = [
  {
    title: "1. Acceptance of Terms",
    paragraphs: [
      "By creating an account, signing in, or using NyxID Mobile (the \"App\"), you agree to these Terms of Service and all applicable laws. If you do not agree, do not use the App.",
    ],
  },
  {
    title: "2. Description of Service",
    paragraphs: [
      "NyxID Mobile is a security authenticator that receives approval challenges from NyxID-connected services and allows you to approve, deny, or revoke access requests from your mobile device.",
    ],
    bullets: [
      "Receive push notifications for high-risk approval requests",
      "Review challenge details and approve or deny access",
      "Manage and revoke previously approved sessions",
      "Sign in with Google, GitHub, or Apple",
    ],
  },
  {
    title: "3. Account and Security",
    paragraphs: [
      "You are responsible for maintaining the security of your account and device. All actions taken through your authenticated session are attributed to you.",
    ],
    bullets: [
      "Use accurate information when signing in",
      "Keep your device secure and up to date",
      "Promptly report unauthorized access to your account",
      "Do not share your session or allow others to act on your behalf",
    ],
  },
  {
    title: "4. Sign in with Apple",
    paragraphs: [
      "When you choose to sign in with Apple, we receive your Apple ID email (or a private relay address if you choose to hide your email) and a verified identity token. We do not receive your Apple ID password. Apple's terms of service and privacy policy also apply to your use of Sign in with Apple.",
    ],
  },
  {
    title: "5. Push Notifications",
    paragraphs: [
      "The App uses the push notification services supported by your device platform (such as FCM on Android or APNs on iOS) to deliver time-sensitive approval requests. You may disable notifications in your device settings, but this may prevent you from receiving approval challenges in real time.",
    ],
  },
  {
    title: "6. Permitted Use",
    paragraphs: ["The App may be used only for lawful identity and access management purposes."],
    bullets: [
      "Responding to authentication and approval challenges",
      "Managing authorized sessions and revocations",
      "Reviewing security audit information",
    ],
  },
  {
    title: "7. Prohibited Conduct",
    bullets: [
      "Attempting to bypass, tamper with, or exploit security controls",
      "Automated, abusive, or denial-of-service interactions",
      "Using the App to facilitate violations of law, rights, or contracts",
      "Reverse engineering or decompiling the App except where permitted by law",
    ],
    paragraphs: [],
  },
  {
    title: "8. Third-Party Services",
    paragraphs: [
      "The App connects to third-party identity providers (Google, GitHub, Apple) for authentication. Their respective terms and privacy policies apply to your use of those services. NyxID is not responsible for third-party service availability or data practices.",
    ],
  },
  {
    title: "9. Account Deletion",
    paragraphs: [
      "You may permanently delete your account and all associated server-side data from the Account Settings screen within the App. This action is irreversible. Upon deletion, your sessions are revoked and personal data is removed in accordance with our Privacy Policy. If you sign in again with the same provider after deletion, a new account will be created; your previous data will not be restored.",
    ],
  },
  {
    title: "10. Availability and Changes",
    paragraphs: [
      "We may update, suspend, or discontinue features at any time to improve security, stability, or compliance. We will update the effective date when material changes are made to these terms.",
    ],
  },
  {
    title: "11. Disclaimers",
    paragraphs: [
      "The App is provided on an \"as is\" and \"as available\" basis to the extent permitted by law, without warranties of merchantability, fitness for a particular purpose, or uninterrupted operation.",
    ],
  },
  {
    title: "12. Limitation of Liability",
    paragraphs: [
      "To the maximum extent permitted by law, NyxID and its operators are not liable for indirect, incidental, special, or consequential damages arising from your use of the App.",
    ],
  },
  {
    title: "13. Governing Law",
    paragraphs: [
      "These terms are governed by the laws of the jurisdiction in which the service operator is established, without regard to conflict of law provisions.",
    ],
  },
  {
    title: "14. Contact",
    paragraphs: ["For questions about these terms: legal@chrono-ai.fun"],
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
