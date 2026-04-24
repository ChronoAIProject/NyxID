import { useMemo } from "react";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ScrollView, StyleSheet, Text, View, type ViewStyle } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";

import { BlurBackButton } from "../../components/BlurBackButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "PrivacyPolicy">;

type LegalSection = {
  title: string;
  paragraphs: string[];
  bullets?: string[];
};

const EFFECTIVE_DATE = "2026-03-11";

const PRIVACY_SECTIONS: LegalSection[] = [
  {
    title: "1. Introduction",
    paragraphs: [
      "NyxID Mobile (the \"App\") is a security authenticator for the NyxID identity platform. This Privacy Policy explains what data we collect, how we use it, and how we protect it.",
      "By using the App, you agree to the practices described in this policy.",
    ],
  },
  {
    title: "2. Information We Collect",
    paragraphs: ["We collect the minimum data necessary to provide secure authentication and approval services."],
    bullets: [
      "Account identity: email address (or Apple private relay address), display name, and user ID from your chosen sign-in provider (Google, GitHub, or Apple)",
      "Authentication tokens: access tokens and refresh tokens stored securely using Expo Secure Store and the protected storage provided by your device platform",
      "Device information: push notification token (such as FCM on Android or APNs on iOS), device platform, and app identifier for delivering approval challenges",
      "Usage data: approval decisions (approve/deny/revoke), timestamps, and idempotency keys for security audit trails",
      "Server-side only: our servers may receive IP address and request headers (e.g. user-agent) as part of normal HTTPS requests. The app does not collect, store, or share this technical metadata.",
    ],
  },
  {
    title: "3. How We Use Your Information",
    bullets: [
      "Authenticate your identity and maintain your session",
      "Deliver push notifications for time-sensitive approval challenges",
      "Process your approval, denial, and revocation decisions",
      "Register and manage your device for push delivery",
      "Maintain security audit logs for compliance and abuse prevention",
      "Refresh expired sessions automatically to minimize re-authentication",
    ],
    paragraphs: [],
  },
  {
    title: "4. Sign in with Apple",
    paragraphs: [
      "If you sign in with Apple, we receive a verified identity token and your email address (or Apple's private relay address if you choose \"Hide My Email\"). We do not receive your Apple ID password or any data beyond what Apple provides through its identity service. You may manage your Sign in with Apple connections in your Apple ID settings.",
    ],
  },
  {
    title: "5. Push Notifications",
    paragraphs: [
      "We use the push notification services supported by your device platform (such as FCM on Android or APNs on iOS) to deliver approval challenges. Your device push token is registered with our server upon login and removed upon sign-out or account deletion.",
      "Push notification payloads contain only minimal identifiers (challenge ID). Sensitive details are fetched separately over an authenticated API connection.",
    ],
  },
  {
    title: "6. Data Storage and Security",
    paragraphs: [
      "Authentication tokens are stored using Expo Secure Store and the secure storage protections provided by your operating system.",
      "All network communication uses TLS encryption. Sensitive server-side fields are encrypted with AES-256.",
      "Access tokens have scoped expiry. Refresh tokens are rotated and can be revoked at any time.",
    ],
  },
  {
    title: "7. Data Sharing",
    paragraphs: ["We do not sell, rent, or trade your personal data. Data may be shared only in the following circumstances:"],
    bullets: [
      "With third-party identity providers (Google, GitHub, Apple) as part of the authentication flow you initiated",
      "When required by applicable law, regulation, or legal process",
      "To protect the security and integrity of our services against fraud or abuse",
    ],
  },
  {
    title: "8. Data Retention",
    paragraphs: [
      "Account data is retained while your account is active.",
      "When you delete your account (available in Account Settings), all personal data and server-side records are permanently removed. Security audit logs may be retained for a limited period as required for compliance.",
      "If you sign in again with the same provider (e.g. Apple, Google, GitHub) after deletion, a new account will be created; your previous data will not be restored.",
      "Push tokens are removed from our server when you sign out or delete your account.",
    ],
  },
  {
    title: "9. Your Rights",
    paragraphs: ["You have the right to:"],
    bullets: [
      "Access the data associated with your account",
      "Delete your account and all server-side data permanently from within the App",
      "Revoke any active approval grants at any time",
      "Disconnect third-party sign-in providers",
      "Disable push notifications through your device settings",
    ],
  },
  {
    title: "10. Local Storage",
    paragraphs: [
      "The App stores authentication tokens and push token references using Expo Secure Store and platform-protected local storage. The App does not use tracking cookies, advertising identifiers, or cross-app tracking.",
    ],
  },
  {
    title: "11. Product Analytics",
    paragraphs: [
      "When you opt in (and only when you opt in), the App uses a product-analytics service (PostHog) to record anonymized usage events such as which screens you visit, which buttons you tap, and whether push notifications are received. Events carry your NyxID user identifier so we can measure funnels like sign-up to first approval. We strip persistent hardware identifiers (device id, device name) before any event leaves the device. Raw deep-link URLs, notification bodies, and any tokens are never captured.",
      "You can toggle analytics on or off at any time from Settings. Turning it off clears your local identifier and stops new events from this device immediately. NyxID's backend may still emit events attributable to your account while you use the service (for example, when your app completes an approval or registers a push token); account-level opt-out for backend events is tracked as a follow-up. Requesting account deletion also fires a deletion signal to the analytics provider so historical events tied to your identifier are removed.",
      "Self-hosters of NyxID can run the App with analytics disabled by default, or point it at their own analytics project. The App never transmits events to any third party when consent is off.",
    ],
  },
  {
    title: "12. Children's Privacy",
    paragraphs: [
      "The App is not intended for use by children under 16 (or the applicable minimum age in your jurisdiction). We do not knowingly collect data from children. If you believe a child has provided data to us, please contact us for removal.",
    ],
  },
  {
    title: "13. Policy Updates",
    paragraphs: [
      "We may update this policy to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date. Continued use of the App after changes constitutes acceptance.",
    ],
  },
  {
    title: "14. Contact",
    paragraphs: ["Privacy inquiries: privacy@chrono-ai.fun"],
  },
];

export function PrivacyPolicyScreen({ navigation }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);

  return (
    <ScreenContainer>
      <View style={styles.stickyBack}>
        <BlurBackButton
          onPress={() => {
            capture({
              name: "ui.mobile_nav_target_opened",
              props: { target: "privacy_back", source: "back" },
            });
            navigation.goBack();
          }}
        />
      </View>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra, { paddingHorizontal: spacing.xxl }]}
        showsVerticalScrollIndicator={false}
      >
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

      </ScrollView>
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  stickyBack: {
    position: "absolute",
    top: spacing.xxl,
    left: spacing.xxl,
    zIndex: 10,
  } satisfies ViewStyle,
  scrollContentExtra: {
    paddingTop: 64,
    paddingBottom: spacing.xxxl,
  },
  sectionWrap: {
    gap: spacing.xs,
  },
  sectionTitle: {
    color: c.textPrimary,
    ...typeScale.bodyStrong,
  },
  sectionBody: {
    color: c.textSecondary,
    ...typeScale.caption,
    lineHeight: 18,
  },
  bulletBody: {
    color: c.textSecondary,
    ...typeScale.caption,
    lineHeight: 18,
  },
});
