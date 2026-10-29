import { memo, useCallback, useContext, useMemo, useRef } from "react";
import {
  AccessibilityInfo,
  findNodeHandle,
  Linking,
  ScrollView,
  Text,
  View,
} from "react-native";
import Markdown, {
  MarkdownIt,
  renderRules,
  type RenderRules,
} from "react-native-markdown-display";
import * as WebBrowser from "expo-web-browser";
import { useTheme } from "../../theme/ThemeContext";
import { TRANSCRIPT_SIDE_PADDING } from "../display/chatLayout";
import { fonts, radius } from "../../theme/designTokens";
import { assistantSelectableMarkdown } from "./assistantSelectableMarkdown";
import { AssistantLinkContext } from "./AssistantLinkContext";
import { assistantLinkTarget, assistantLinkLabel } from "./assistantLinkTarget";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import {
  assistantMarkdownOptions,
  configureAssistantMarkdown,
  safeAssistantLinkUrl,
} from "./assistantMarkdown";

const markdown = configureAssistantMarkdown(
  MarkdownIt(assistantMarkdownOptions),
);
function openLink(url: string) {
  const safe = safeAssistantLinkUrl(url);
  if (safe) void WebBrowser.openBrowserAsync(safe).catch(() => undefined);
  return false;
}

/** Stable message bodies avoid reparsing earlier replies on every composer edit. */
export const AssistantMarkdownBody = memo(function AssistantMarkdownBody({
  text,
  variant = "private",
  onLinkPress = openLink,
}: {
  text: string;
  variant?: "private" | "crew";
  onLinkPress?: (url: string) => boolean;
}) {
  const { colors } = useTheme();
  const linkHost = useContext(AssistantLinkContext);
  const anchors = useRef(new Map<string, Text>());
  const openHref = useCallback(
    (url: string, trigger?: number) => {
      const safe = safeAssistantLinkUrl(url);
      if (!safe) return false;
      if (safe.startsWith("#")) {
        const target = anchors.current.get(safe.slice(1));
        if (target)
          requestAnimationFrame(() => {
            if (anchors.current.get(safe.slice(1)) !== target) return;
            const handle = findNodeHandle(target);
            if (handle) AccessibilityInfo.setAccessibilityFocus(handle);
          });
        return false;
      }
      if (safe.startsWith("mailto:")) {
        void Linking.openURL(safe).catch(() => undefined);
        return false;
      }
      if (linkHost?.(safe, trigger)) return false;
      return onLinkPress(safe);
    },
    [onLinkPress, linkHost],
  );
  const markdownStyle = useMemo(
    () => ({
      body: {
        color: colors.textPrimary,
        fontFamily: fonts.bodyRegular,
        fontSize: 14,
        lineHeight: 23,
      },
      paragraph: { marginTop: 0, marginBottom: 0 },
      heading1: {
        fontFamily: fonts.displaySemi,
        fontSize: 22,
        lineHeight: 29,
        color: colors.textPrimary,
      },
      heading2: {
        fontFamily: fonts.displaySemi,
        fontSize: 18,
        lineHeight: 25,
        color: colors.textPrimary,
      },
      link: {
        color: colors.primaryOnTint,
        textDecorationLine: "underline" as const,
      },
      code_inline: {
        fontFamily: fonts.mono,
        backgroundColor: colors.cardSoft,
        color: colors.textPrimary,
      },
      fence: {
        fontFamily: fonts.mono,
        fontSize: 12,
        backgroundColor: colors.cardSoft,
        color: colors.textPrimary,
        borderColor: colors.border,
        borderRadius: radius.md,
      },
      blockquote: {
        backgroundColor: colors.cardSoft,
        borderColor: colors.primary,
        color: colors.textSecondary,
      },
      table: { borderColor: colors.border },
      tr: { borderColor: colors.border },
      th: { paddingHorizontal: 12, paddingVertical: 8, minWidth: 140 },
      td: { paddingHorizontal: 12, paddingVertical: 8, minWidth: 140 },
    }),
    [colors],
  );

  const crewStyle = useMemo(
    () => ({
      body: {
        color: colors.textPrimary,
        fontSize: 14,
        lineHeight: 23,
        fontFamily: fonts.bodyRegular,
      },
      paragraph: { marginTop: 0, marginBottom: 0 },
      link: { color: colors.primaryOnTint },
      fence: {
        backgroundColor: colors.cardSoft,
        color: colors.textPrimary,
        borderColor: colors.border,
        fontFamily: fonts.mono,
      },
      code_inline: {
        backgroundColor: colors.cardSoft,
        color: colors.textPrimary,
      },
      heading1: { fontSize: 22, color: colors.textPrimary },
      heading2: { fontSize: 18, color: colors.textPrimary },
    }),
    [colors],
  );
  const combinedCrewStyle = useMemo(
    () => ({ ...markdownStyle, ...crewStyle }),
    [markdownStyle, crewStyle],
  );
  const rules: RenderRules = useMemo(
    () => ({
      ...assistantSelectableMarkdown,
      body: (node, children, _parents, styles) => (
        <View key={node.key} style={styles._VIEW_SAFE_body}>
          {children.map((child, index) => {
            const type = node.children[index]?.type ?? "";
            const previous = node.children[index - 1]?.type ?? "";
            const gap =
              index === 0
                ? 0
                : type === "heading1"
                  ? 20
                  : type.startsWith("heading")
                    ? 16
                    : previous.startsWith("heading")
                      ? 8
                      : 12;
            return (
              <View
                key={node.children[index]?.key ?? index}
                style={{ marginTop: gap }}
              >
                {child}
              </View>
            );
          })}
        </View>
      ),
      link: (node, children) => {
        const target = assistantLinkTarget(
          node.attributes.href,
          NYXID_FRONTEND_URL,
        );
        const label = node.children
          .map((child) => child.content ?? "")
          .join("");
        return (
          <Text
            key={node.key}
            accessibilityRole={target?.kind === "connect" ? "button" : "link"}
            style={
              target?.kind === "connect"
                ? [
                    markdownStyle.link,
                    {
                      backgroundColor: colors.primaryTone.bg,
                      paddingHorizontal: 8,
                      paddingVertical: 6,
                      borderRadius: 6,
                    },
                  ]
                : markdownStyle.link
            }
            onPress={(event) => {
              const handle = Number(event?.nativeEvent?.target);
              openHref(
                node.attributes.href,
                Number.isFinite(handle) ? handle : undefined,
              );
            }}
          >
            {target?.kind === "connect" || target?.kind === "channel"
              ? assistantLinkLabel(target, label)
              : children}
          </Text>
        );
      },
      image: (node) => {
        const safe = safeAssistantLinkUrl(node.attributes.src ?? "");
        return (
          <Text
            key={node.key}
            accessibilityRole={safe ? "link" : undefined}
            style={{
              color: safe ? colors.primaryOnTint : colors.textSecondary,
            }}
            onPress={safe ? () => openHref(safe) : undefined}
          >
            {typeof node.attributes.alt === "string" &&
            node.attributes.alt.trim()
              ? node.attributes.alt
              : "Image"}
          </Text>
        );
      },
      list_item: (node, children, ...args) => {
        const task = node.attributes["data-task-checked"];
        if (task === undefined)
          return renderRules.list_item?.(node, children, ...args);
        const checked = task === "true";
        return (
          <View
            key={node.key}
            style={{ flexDirection: "row", gap: 6, alignItems: "flex-start" }}
          >
            <View
              accessibilityRole="checkbox"
              accessibilityLabel="Task"
              accessibilityState={{ checked, disabled: true }}
              style={{
                width: 15,
                height: 15,
                marginTop: 4,
                borderWidth: 1,
                borderRadius: 3,
                alignItems: "center",
                justifyContent: "center",
                borderColor: checked ? colors.primaryOnTint : colors.textMuted,
                backgroundColor: checked
                  ? colors.primaryTone.bg
                  : "transparent",
              }}
            >
              {checked ? (
                <Text
                  accessible={false}
                  style={{
                    color: colors.primaryOnTint,
                    fontSize: 11,
                    lineHeight: 13,
                  }}
                >
                  ✓
                </Text>
              ) : null}
            </View>
            <View style={{ flex: 1 }}>{children}</View>
          </View>
        );
      },
      table: (node, children) => {
        const header = node.children.find((row) => row.type === "thead")
          ?.children[0];
        const columns = header?.children.length ?? 1;
        return (
          <ScrollView
            key={node.key}
            horizontal
            accessibilityLabel="Table"
            contentContainerStyle={{
              flexGrow: 1,
              paddingRight: TRANSCRIPT_SIDE_PADDING,
            }}
            style={{ maxWidth: "100%" }}
          >
            <View
              style={{
                flexGrow: 1,
                minWidth: columns * 140,
                borderColor: colors.border,
                borderWidth: 1,
                borderRadius: radius.md,
                overflow: "hidden",
              }}
            >
              {children}
            </View>
          </ScrollView>
        );
      },
      th: (node, children) => (
        <View
          key={node.key}
          style={{
            flex: 1,
            minWidth: 140,
            paddingHorizontal: 12,
            paddingVertical: 8,
            backgroundColor: colors.cardSoft,
          }}
        >
          {children}
        </View>
      ),
      td: (node, children) => (
        <View
          key={node.key}
          style={{
            flex: 1,
            minWidth: 140,
            paddingHorizontal: 12,
            paddingVertical: 8,
          }}
        >
          {children}
        </View>
      ),
      textgroup: (node, children, parents, styles, ...rest) => {
        const cell = parents.find(
          (parent) => parent.type === "th" || parent.type === "td",
        );
        if (!cell)
          return assistantSelectableMarkdown.textgroup?.(
            node,
            children,
            parents,
            styles,
            ...rest,
          );
        const align = /text-align:(left|center|right)/.exec(
          cell.attributes.style ?? "",
        )?.[1] as "left" | "center" | "right" | undefined;
        return (
          <Text
            key={node.key}
            selectable
            style={[styles.textgroup, { textAlign: align }]}
          >
            {children}
          </Text>
        );
      },
      footnote_block: (node, children) => (
        <View
          key={node.key}
          style={{
            borderTopWidth: 1,
            borderColor: colors.border,
            paddingTop: 8,
          }}
        >
          <Text
            accessibilityRole="header"
            style={{
              color: colors.textSecondary,
              fontFamily: fonts.body,
              fontSize: 11,
              marginBottom: 4,
            }}
          >
            Footnotes
          </Text>
          {children}
        </View>
      ),
      footnote: (node, children) => (
        <View key={node.key} style={{ flexDirection: "row", gap: 6 }}>
          <Text
            ref={(target) => {
              if (target) anchors.current.set(node.attributes.id, target);
              else anchors.current.delete(node.attributes.id);
            }}
            accessibilityLabel={`Footnote ${node.attributes["data-number"]}`}
            style={{ color: colors.textSecondary, fontSize: 11 }}
          >
            {node.attributes["data-number"]}.
          </Text>
          <View style={{ flex: 1 }}>{children}</View>
        </View>
      ),
      footnote_ref: (node) => (
        <Text
          key={node.key}
          ref={(target) => {
            if (target) anchors.current.set(node.attributes.id, target);
            else anchors.current.delete(node.attributes.id);
          }}
          accessibilityRole="link"
          accessibilityLabel={`Footnote ${node.attributes["data-number"]}`}
          onPress={() => openHref(node.attributes.href)}
          style={{ color: colors.primaryOnTint, fontSize: 10 }}
        >
          [{node.attributes["data-number"]}]
        </Text>
      ),
      footnote_anchor: (node) => (
        <Text
          key={node.key}
          accessibilityRole="link"
          accessibilityLabel={`Back to footnote ${node.attributes["data-number"]} reference`}
          onPress={() => openHref(node.attributes.href)}
          style={{ color: colors.primaryOnTint, fontSize: 12 }}
        >
          {" "}
          ↩
        </Text>
      ),
    }),
    [markdownStyle, colors, openHref],
  );
  return (
    <Markdown
      markdownit={markdown}
      mergeStyle
      style={variant === "crew" ? combinedCrewStyle : markdownStyle}
      rules={rules}
      onLinkPress={onLinkPress}
    >
      {text}
    </Markdown>
  );
});
