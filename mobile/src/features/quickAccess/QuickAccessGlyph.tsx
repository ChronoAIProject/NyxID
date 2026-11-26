import {
  Bell,
  Bot,
  CalendarClock,
  MessagesSquare,
  Monitor,
  Plug,
  ScanQrCode,
  Settings,
  ShieldCheck,
  SquarePen,
  Terminal,
  UserPlus,
  Users,
  type LucideIcon,
} from "lucide-react-native";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import type { QuickAccessDestination } from "../../lib/api/quickAccessSchema";
import type { QuickAccessEntry } from "./quickAccessModel";

/** The side menu's icon for each destination (Agents and Crews have none there). */
export const QUICK_ACCESS_ICONS: Record<QuickAccessDestination, LucideIcon> = {
  new_chat: SquarePen,
  notifications: Bell,
  scan_qr: ScanQrCode,
  terminal_login_code: Terminal,
  recent_chats: MessagesSquare,
  automations: CalendarClock,
  machines: Monitor,
  plugins: Plug,
  activity: ShieldCheck,
  create_agent: UserPlus,
  settings: Settings,
  agents: Bot,
  crews: Users,
};

/**
 * One quick-access entry's mark at the rail's shortcut size: the side menu's
 * icon, an agent's avatar, or a crew's first two members. `surface` is the
 * colour the crew's avatar rings cut out against.
 */
export function QuickAccessGlyph({
  entry,
  color,
  surface,
  size = 20,
}: {
  entry: QuickAccessEntry;
  color: string;
  surface: string;
  size?: number;
}) {
  if (entry.kind === "agent")
    return <AgentAvatar agent={entry.agent} size="sm" />;
  if (entry.kind === "crew")
    return (
      <AgentAvatarStack
        agents={entry.members}
        size="xs"
        max={2}
        surface={surface}
      />
    );
  const Icon = QUICK_ACCESS_ICONS[entry.id];
  return <Icon size={size} color={color} />;
}
