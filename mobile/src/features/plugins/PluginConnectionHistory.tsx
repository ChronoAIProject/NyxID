import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ChevronsUpDown, SlidersHorizontal } from "lucide-react-native";
import type {
  ContentScreenProps,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  ActionButton,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { ApiError } from "../../lib/api/ApiError";
import { pluginConnectionHistoryApi as api } from "../../lib/api/pluginConnectionHistoryApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  historyActor,
  historyChange,
  historySummary,
} from "./pluginConnectionHistoryTypes";

const ROOT = ["workspace", "services", "history"] as const;
const time = (value: string) => {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? "Time not recorded"
    : date.toLocaleString();
};

/** Embedded in the connection's History tab; all records come from the Web API. */
export function PluginConnectionHistory({ serviceId }: { serviceId: string }) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const client = useQueryClient();
  const epoch = getAuthEpoch();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const selectionScope = JSON.stringify([epoch, serviceId]);
  const [selection, setSelection] = useState({
    scope: selectionScope,
    actions: [] as string[],
  });
  const actions = selection.scope === selectionScope ? selection.actions : [];
  const [filterOpen, setFilterOpen] = useState(false);
  const active = focused && foreground;
  const query = useInfiniteQuery({
    queryKey: [...ROOT, epoch, serviceId, actions],
    queryFn: ({ pageParam, signal }) =>
      api.page(serviceId, pageParam, actions, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (page, _pages, _last, params) =>
      page.next_cursor !== null && !params.includes(page.next_cursor)
        ? page.next_cursor
        : undefined,
    enabled: active && !!serviceId,
    staleTime: 0,
    gcTime: 0,
    retry: false,
    refetchOnMount: "always",
  });
  const options = useQuery({
    queryKey: [...ROOT, epoch, "actions"],
    queryFn: ({ signal }) => api.actions(signal),
    enabled: active && filterOpen,
    retry: false,
    staleTime: 0,
    gcTime: 0,
  });
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  useEffect(() => {
    setFilterOpen(false);
  }, [selectionScope, active]);
  useEffect(() => {
    setSelection({ scope: selectionScope, actions: [] });
  }, [selectionScope]);
  useEffect(() => {
    if (!active)
      void client.cancelQueries({ queryKey: [...ROOT, epoch, serviceId] });
    return () => {
      void client.cancelQueries({ queryKey: [...ROOT, epoch, serviceId] });
    };
  }, [active, client, epoch, serviceId]);
  // A failed authorization/context refresh must hide any previously cached rows.
  const pages = query.isError ? undefined : query.data?.pages;
  const first = pages?.[0];
  const groups =
    pages
      ?.flatMap((page) => page.groups)
      .filter((group) => group.events.length) ?? [];
  const unavailable =
    query.error instanceof ApiError &&
    [403, 404].includes(query.error.statusCode);
  const rows = groups.flatMap((group) => {
    const summary = historySummary(group);
    return summary ? [{ group, summary }] : [];
  });
  return (
    <View style={styles.stack}>
      <SettingsGroup footer="Metadata only. Configuration changes and authorship are retained. Credentials are never shown.">
        <SettingsRow
          icon={SlidersHorizontal}
          label="Actions"
          value={
            actions.length ? `${actions.length} action filters` : "All actions"
          }
          accessibilityLabel="Filter history actions"
          accessibilityHint="Opens a list of actions to include"
          accessibilityState={{ expanded: filterOpen }}
          accessory={<ChevronsUpDown size={16} color={colors.textMuted} />}
          onPress={() => setFilterOpen((value) => !value)}
        />
      </SettingsGroup>
      {filterOpen && (
        <WorkspaceSelectionSheet
          label="History actions"
          title="Filter history actions"
          description="Choose actions to include. Clear the selection to show all actions."
          options={options.isError ? [] : (options.data ?? [])}
          value={actions}
          multiple
          allowEmpty
          scopeKey={selectionScope}
          readOnly={options.isPending || options.isError || !active}
          onChange={(values) =>
            setSelection({ scope: selectionScope, actions: [...values].sort() })
          }
          onClose={() => setFilterOpen(false)}
        >
          {options.isPending && (
            <SkeletonRows
              rows={4}
              icon={false}
              detail={false}
              accessibilityLabel="Loading history filters"
            />
          )}
          {options.isError && (
            <>
              <Notice message="History filters could not be loaded." />
              <ActionButton
                label="Retry filters"
                variant="secondary"
                disabled={!active}
                onPress={() => void options.refetch()}
              />
            </>
          )}
        </WorkspaceSelectionSheet>
      )}
      {query.isPending && (
        <SkeletonRows
          rows={4}
          icon={false}
          title
          accessibilityLabel="Loading connection history"
        />
      )}
      {query.isError && (
        <>
          <Notice
            tone="error"
            message={
              unavailable
                ? "Connection history is unavailable. Personal owners and currently permitted organization admins can view it."
                : "Connection history could not be loaded."
            }
          />
          <SettingsGroup inset={false}>
            <SettingsRow
              label="Retry history"
              tone="action"
              disabled={!active}
              onPress={() => void query.refetch()}
            />
          </SettingsGroup>
        </>
      )}
      {first?.deleted && (
        <Notice message="This connection was deleted. Its recorded history is retained." />
      )}
      {first?.legacy && (
        <Notice
          message={`Creator and earlier edits may not have been recorded.${first.tracked_since ? ` Tracking since ${time(first.tracked_since)}.` : ""}`}
        />
      )}
      {first && !rows.length && (
        <Text style={[styles.note, { color: colors.textMuted }]}>
          {actions.length
            ? "No changes match these filters."
            : "No recorded changes yet."}
        </Text>
      )}
      {rows.length ? (
        <SettingsGroup
          title="Changes"
          inset={false}
          footer="Deleted or unavailable connections may still retain a readable history."
        >
          {rows.map(({ group, summary }) => {
            const title = summary.action_label || "Connection updated";
            return (
              <SettingsRow
                key={`${serviceId}:${group.id}`}
                label={title}
                detail={`${historyActor(summary.actor)} · ${time(group.events.at(-1)!.committed_at)}${group.events.length > 1 ? ` · ${group.events.length} changes` : ""}`}
                accessibilityLabel={`${title}, ${historyActor(summary.actor)}`}
                onPress={() =>
                  navigation.push("ConnectionHistoryDetail", {
                    title,
                    group,
                  })
                }
              />
            );
          })}
          {query.hasNextPage && !query.isError ? (
            <SettingsRow
              label="Load more history"
              tone="action"
              disabled={!active || query.isFetchingNextPage}
              accessory={
                query.isFetchingNextPage ? (
                  <ActivityIndicator color={colors.primary} />
                ) : undefined
              }
              onPress={() => void query.fetchNextPage()}
            />
          ) : null}
        </SettingsGroup>
      ) : (
        <Text style={[styles.note, { color: colors.textMuted }]}>
          Deleted or unavailable connections may still retain a readable
          history.
        </Text>
      )}
    </View>
  );
}

/** One recorded change set: every event as label/value rows, metadata only. */
export function ConnectionHistoryDetailScreen({
  route,
}: ContentScreenProps<"ConnectionHistoryDetail">) {
  const { group } = route.params;
  return (
    <WorkspacePage
      title={route.params.title}
      showHeading={false}
      contentStyle={styles.grouped}
    >
      {group.events.map((event) => (
        <View key={event.id} style={styles.event}>
          <SettingsGroup
            title={event.action_label || "Connection updated"}
            inset={false}
            footer={
              event.additional_changes
                ? "Other service settings updated. Additional change details are unavailable."
                : undefined
            }
          >
            <SettingsRow label="Changed by" value={historyActor(event.actor)} />
            <SettingsRow label="Time" value={time(event.committed_at)} />
            {event.actor.api_key_id ? (
              <SettingsRow label="Authenticated" value="API key" />
            ) : null}
            {event.actor.app_id ? (
              <SettingsRow label="Application" value={event.actor.app_id} />
            ) : null}
            {event.changes.map((change, index) => (
              <SettingsRow
                key={`${change.field}:${index}`}
                label={change.label || change.field}
                value={historyChange(change)}
              />
            ))}
          </SettingsGroup>
          {event.audit_status === "mismatch" && (
            <Notice
              tone="error"
              message="This record differs from its audit mirror."
            />
          )}
        </View>
      ))}
    </WorkspacePage>
  );
}

const styles = StyleSheet.create({
  stack: { gap: spacing.huge },
  grouped: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.sm,
    paddingBottom: spacing.huge * 2,
    gap: spacing.huge,
  },
  event: { gap: spacing.md },
  note: { ...typeScale.small, paddingHorizontal: spacing.xxl },
});
