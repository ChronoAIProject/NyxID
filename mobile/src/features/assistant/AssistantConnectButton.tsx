import { useEffect, useRef, useState } from "react";
import { findNodeHandle, Pressable, Text, View } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight, Check } from "lucide-react-native";
import { ServiceIcon } from "../../components/ServiceIcon";
import { connectLinksApi } from "../../lib/api/connectLinksApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { firstPartyFrontendUrls } from "../../lib/env";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { assistantLinkTarget } from "./assistantLinkTarget";
import { connectButtonState } from "./connectButtonState";

/**
 * A hosted connect link from a reply, shown as a real button: it names the
 * service, says how long the link lasts and switches off once the link is
 * used, expired or cancelled. Add-service links keep the reply's wording.
 */
export function AssistantConnectButton({
  label,
  href,
  onPress,
}: {
  label: string;
  href?: string;
  onPress: (trigger?: number) => void;
}) {
  const { colors } = useTheme();
  const anchor = useRef<View>(null);
  const focused = useIsFocused();
  const target = href ? assistantLinkTarget(href, firstPartyFrontendUrls()) : null;
  const token = target?.kind === "connect" ? target.token : undefined;
  const preview = useQuery({
    queryKey: ["assistant", "connect-link-preview", getAuthEpoch(), token],
    queryFn: ({ signal }) => connectLinksApi.preview(token!, signal),
    enabled: !!token && focused,
    retry: false,
    staleTime: 10_000,
    // Follow a waiting link so the button switches off when it is used or lapses.
    refetchInterval: (query) =>
      query.state.data?.status === "pending" ? 15_000 : false,
  });
  // Minute ticks keep the expiry countdown honest between checks.
  const [now, setNow] = useState(() => Date.now());
  const waiting = preview.data?.status === "pending";
  useEffect(() => {
    if (!waiting) return;
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, [waiting]);
  const state = connectButtonState(label, preview.data, now);
  const slug =
    preview.data?.service_slug ??
    (target?.kind === "service" ? target.slug : undefined);
  return (
    <View ref={anchor} style={{ marginTop: 10, alignSelf: "stretch", gap: 6 }}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={state.label}
        accessibilityHint={state.disabled ? state.detail : "Opens a secure NyxID page"}
        accessibilityState={{ disabled: state.disabled }}
        disabled={state.disabled}
        onPress={() => {
          const handle = anchor.current ? findNodeHandle(anchor.current) : null;
          onPress(handle ?? undefined);
        }}
        style={({ pressed }) => ({
          minHeight: 48,
          flexDirection: "row",
          alignItems: "center",
          gap: 10,
          paddingHorizontal: 14,
          borderRadius: 12,
          borderWidth: 1,
          borderColor: colors.controlBorder,
          backgroundColor: colors.card,
          opacity: state.disabled ? 0.55 : pressed ? 0.75 : 1,
          shadowColor: "#000000",
          shadowOpacity: state.disabled ? 0 : 0.08,
          shadowRadius: 3,
          shadowOffset: { width: 0, height: 1 },
        })}
      >
        {slug ? <ServiceIcon slug={slug} size="md" color={colors.textPrimary} /> : null}
        <Text
          numberOfLines={1}
          style={{
            flex: 1,
            fontFamily: fonts.bodySemi,
            fontSize: 15,
            lineHeight: 20,
            color: colors.textPrimary,
          }}
        >
          {state.label}
        </Text>
        {state.tone === "done" ? (
          <Check size={18} color={colors.success} />
        ) : state.disabled ? null : (
          <ArrowUpRight size={18} color={colors.textSecondary} />
        )}
      </Pressable>
      {state.detail ? (
        <Text
          accessibilityLiveRegion="polite"
          style={{
            fontFamily: fonts.body,
            fontSize: 12,
            lineHeight: 17,
            color: state.tone === "done" ? colors.success : colors.textSecondary,
          }}
        >
          {state.detail}
        </Text>
      ) : null}
    </View>
  );
}
