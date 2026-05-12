import Constants from "expo-constants";
import { useEffect, useMemo, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View, type ViewStyle } from "react-native";
import Markdown from "react-native-markdown-display";

import { BlurBackButton } from "../../components/BlurBackButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { spacing, typeScale } from "../../theme/designTokens";

/**
 * Render a legal document (privacy / terms) by fetching the canonical
 * markdown from the frontend deploy. Same .md files used by the web
 * dashboard, so both surfaces always show the same text.
 *
 * Source of truth: frontend/public/legal/{privacy,terms}.md
 * Served at: <LEGAL_BASE_URL>/legal/<doc>.md
 *
 * `LEGAL_BASE_URL` is per-profile (DEV_/PROD_) in mobile/.env.*.
 */
type Props = {
  title: string;
  docKey: "privacy" | "terms";
  telemetryBackTarget: string;
  onBack: () => void;
};

const FRONT_MATTER_RE = /^---\n([\s\S]*?)\n---\n*/;

function stripFrontMatter(md: string): { content: string; effectiveDate: string | null } {
  const match = md.match(FRONT_MATTER_RE);
  if (!match || !match[0] || !match[1]) return { content: md, effectiveDate: null };
  const body = md.slice(match[0].length);
  const dateMatch = match[1].match(/effective_date:\s*(\S+)/);
  return { content: body, effectiveDate: dateMatch?.[1] ?? null };
}

function resolveDocUrl(docKey: "privacy" | "terms"): string | null {
  const extra = (Constants.expoConfig?.extra ?? {}) as Record<string, unknown>;
  const baseUrl = typeof extra.LEGAL_BASE_URL === "string" ? extra.LEGAL_BASE_URL.replace(/\/$/, "") : "";
  if (!baseUrl) return null;
  return `${baseUrl}/legal/${docKey}.md`;
}

export function LegalDocumentScreen({ title, docKey, telemetryBackTarget, onBack }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const markdownStyles = useMemo(() => createMarkdownStyles(colors), [colors]);

  const url = useMemo(() => resolveDocUrl(docKey), [docKey]);
  const [content, setContent] = useState<string>("");
  const [effectiveDate, setEffectiveDate] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!url) {
      setError("Legal document URL is not configured (LEGAL_BASE_URL).");
      return;
    }
    let cancelled = false;
    fetch(url, { cache: "no-store" })
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.text();
      })
      .then((md) => {
        if (cancelled) return;
        const { content: body, effectiveDate: date } = stripFrontMatter(md);
        // Drop the first H1 since the screen renders `title` in its own header.
        const withoutH1 = body.replace(/^#\s+.+\n+/, "");
        setContent(withoutH1);
        setEffectiveDate(date);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : "Failed to load");
      });
    return () => {
      cancelled = true;
    };
  }, [url]);

  return (
    <ScreenContainer>
      <View style={styles.stickyBack}>
        <BlurBackButton
          onPress={() => {
            capture({
              name: "ui.mobile_nav_target_opened",
              props: { target: telemetryBackTarget, source: "back" },
            });
            onBack();
          }}
        />
      </View>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra, { paddingHorizontal: spacing.xxl }]}
        showsVerticalScrollIndicator={false}
      >
        <Text style={flowStyles.title}>{title}</Text>
        {effectiveDate ? (
          <Text style={flowStyles.subtitle}>Effective date: {effectiveDate}</Text>
        ) : null}

        <View style={flowStyles.card}>
          {error ? (
            <View>
              <Text style={styles.errorTitle}>Unable to load {title.toLowerCase()}</Text>
              <Text style={styles.errorBody}>{error}</Text>
              {url ? <Text style={styles.errorBody}>You can view the latest version at: {url}</Text> : null}
            </View>
          ) : content ? (
            <Markdown style={markdownStyles}>{content}</Markdown>
          ) : (
            <View style={styles.loading}>
              <ActivityIndicator color={colors.textSecondary} />
            </View>
          )}
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
  loading: {
    paddingVertical: spacing.xxl,
    alignItems: "center",
  },
  errorTitle: {
    color: c.textPrimary,
    ...typeScale.bodyStrong,
    marginBottom: spacing.xs,
  },
  errorBody: {
    color: c.textSecondary,
    ...typeScale.caption,
    lineHeight: 18,
    marginBottom: spacing.xs,
  },
});

const createMarkdownStyles = (c: ThemeColors) => ({
  body: {
    color: c.textSecondary,
    ...typeScale.caption,
    lineHeight: 20,
  },
  heading1: {
    color: c.textPrimary,
    ...typeScale.h2,
    marginTop: spacing.lg,
    marginBottom: spacing.sm,
  },
  heading2: {
    color: c.textPrimary,
    ...typeScale.bodyStrong,
    marginTop: spacing.lg,
    marginBottom: spacing.sm,
  },
  heading3: {
    color: c.textPrimary,
    ...typeScale.bodyStrong,
    marginTop: spacing.md,
    marginBottom: spacing.xs,
  },
  paragraph: {
    color: c.textSecondary,
    ...typeScale.caption,
    lineHeight: 20,
    marginBottom: spacing.sm,
  },
  bullet_list: {
    marginBottom: spacing.sm,
  },
  list_item: {
    marginBottom: spacing.xs,
  },
  strong: {
    color: c.textPrimary,
  },
  link: {
    color: c.primary,
  },
  code_inline: {
    color: c.textPrimary,
    ...typeScale.mono,
    backgroundColor: c.cardSoft,
    paddingHorizontal: 4,
    borderRadius: 4,
  },
  hr: {
    backgroundColor: c.borderSoft,
    height: 1,
    marginVertical: spacing.md,
  },
});
