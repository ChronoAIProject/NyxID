import {
  AtSign,
  Calendar,
  ChartNoAxesCombined,
  Code,
  CreditCard,
  FileText,
  Folder,
  Headset,
  Mail,
  Megaphone,
  MessagesSquare,
  PenLine,
  Repeat,
  Search,
  Smartphone,
  Sparkles,
  Store,
  Sunrise,
  Target,
  User,
  Users,
  Zap,
  type LucideIcon,
} from "lucide-react";

const ICONS: Readonly<Record<string, LucideIcon>> = {
  "at-sign": AtSign,
  calendar: Calendar,
  "chart-no-axes-combined": ChartNoAxesCombined,
  code: Code,
  "credit-card": CreditCard,
  "file-text": FileText,
  folder: Folder,
  headset: Headset,
  mail: Mail,
  megaphone: Megaphone,
  "messages-square": MessagesSquare,
  "pen-line": PenLine,
  repeat: Repeat,
  search: Search,
  smartphone: Smartphone,
  sparkles: Sparkles,
  store: Store,
  sunrise: Sunrise,
  target: Target,
  user: User,
  users: Users,
  zap: Zap,
};

/** An icon named by the onboarding API; unknown names show a sparkle. */
export function OnboardingIcon({ name, className }: { readonly name: string; readonly className?: string }) {
  const Icon = ICONS[name] ?? Sparkles;
  return <Icon aria-hidden="true" className={className} />;
}
