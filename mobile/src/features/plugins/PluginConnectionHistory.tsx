import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronRight,
  SlidersHorizontal,
} from "lucide-react-native";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { ApiError } from "../../lib/api/ApiError";
import { pluginConnectionHistoryApi as api } from "../../lib/api/pluginConnectionHistoryApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  historyActor,
  historyChange,
  historySummary,
  type ConnectionHistoryGroup,
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
  return (
    <View style={styles.stack}>
      <Card>
        <Label style={{ color: colors.primaryOnTint, fontFamily: fonts.bodySemi }}>
          ● Metadata only
        </Label>
        <Label style={styles.title}>Connection history</Label>
        <Label muted>
          Configuration changes and authorship are retained. Credentials are
          never shown.
        </Label>
      </Card>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Filter history actions"
        accessibilityState={{ expanded: filterOpen }}
        onPress={() => setFilterOpen((value) => !value)}
        style={[
          styles.row,
          styles.control,
          { backgroundColor: colors.card, borderColor: colors.border },
        ]}
      >
        <SlidersHorizontal size={16} color={colors.textSecondary} />
        <Label style={styles.flex}>
          {actions.length ? `${actions.length} action filters` : "All actions"}
        </Label>
        <ChevronDown size={18} color={colors.textSecondary} />
      </Pressable>
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
            <ActivityIndicator
              color={colors.primary}
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
        <ActivityIndicator
          color={colors.primary}
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
          <ActionButton
            label="Retry history"
            variant="secondary"
            disabled={!active}
            onPress={() => void query.refetch()}
          />
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
      {first && !groups.length && (
        <Label muted>
          {actions.length
            ? "No changes match these filters."
            : "No recorded changes yet."}
        </Label>
      )}
      {groups.map((group) => (
        <HistoryGroup key={`${serviceId}:${group.id}`} group={group} />
      ))}
      {query.hasNextPage && !query.isError && (
        <ActionButton
          label="Load more history"
          variant="secondary"
          disabled={!active}
          loading={query.isFetchingNextPage}
          onPress={() => void query.fetchNextPage()}
        />
      )}
      <Notice message="Deleted or unavailable connections may still retain a readable history." />
    </View>
  );
}

function HistoryGroup({ group }: { group: ConnectionHistoryGroup }) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const summary = historySummary(group);
  if (!summary) return null;
  const title = summary.action_label || "Connection updated";
  return (
    <Card>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${historyActor(summary.actor)}`}
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen((value) => !value)}
        style={styles.row}
      >
        <View style={[styles.stackSmall, styles.flex]}>
          <Label style={styles.title}>{title}</Label>
          <Label muted>{historyActor(summary.actor)}</Label>
          <Label muted style={styles.caption}>
            {time(group.events.at(-1)!.committed_at)}
            {group.events.length > 1 ? ` · ${group.events.length} changes` : ""}
          </Label>
        </View>
        {open ? (
          <ChevronDown size={18} color={colors.textSecondary} />
        ) : (
          <ChevronRight size={18} color={colors.textSecondary} />
        )}
      </Pressable>
      {open &&
        group.events.map((event) => (
          <View
            key={event.id}
            style={[styles.event, { borderColor: colors.border }]}
          >
            <Label style={styles.title}>
              {event.action_label || "Connection updated"}
            </Label>
            <Label muted>
              {historyActor(event.actor)} · {time(event.committed_at)}
            </Label>
            {event.actor.api_key_id && (
              <Label muted>Authenticated API key</Label>
            )}
            {event.actor.app_id && (
              <Label muted>Application {event.actor.app_id}</Label>
            )}
            {event.changes.map((change, index) => (
              <View key={`${change.field}:${index}`} style={styles.stackSmall}>
                <Label>{change.label || change.field}</Label>
                <Label muted selectable>
                  {historyChange(change)}
                </Label>
              </View>
            ))}
            {event.additional_changes && (
              <Label muted>
                Other service settings updated. Additional change details are
                unavailable.
              </Label>
            )}
            {event.audit_status === "mismatch" && (
              <Notice
                tone="error"
                message="This record differs from its audit mirror."
              />
            )}
          </View>
        ))}
    </Card>
  );
}

const styles = StyleSheet.create({
  stack: { gap: 12 },
  stackSmall: { gap: 4 },
  flex: { flex: 1, minWidth: 0 },
  title: { fontSize: 14, fontFamily: fonts.bodySemi },
  caption: { fontSize: 11 },
  row: { flexDirection: "row", alignItems: "center", gap: 10, minHeight: 44 },
  control: { borderWidth: 1, borderRadius: 10, paddingHorizontal: 14 },
  event: { gap: 8, borderTopWidth: 1, paddingTop: 12, marginTop: 4 },
});
