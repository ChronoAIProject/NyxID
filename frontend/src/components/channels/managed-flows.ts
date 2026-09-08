import type { ComponentType } from "react";
import type { ManagedFlow } from "@/lib/channel-platforms";
import { ManagedWhatsApp, ManagedWhatsAppDetail } from "./managed-whatsapp";
import {
  ManagedOAuthConnect,
  ManagedOAuthDetail,
} from "./managed-oauth-connect";
import type {
  ManagedFlowProps,
  ManagedDetailProps,
} from "./managed-flow-types";

export const MANAGED_FLOW_COMPONENTS: Record<
  ManagedFlow,
  {
    Connect: ComponentType<ManagedFlowProps>;
    Detail: ComponentType<ManagedDetailProps>;
  }
> = {
  meta_embedded_signup: {
    Connect: ManagedWhatsApp,
    Detail: ManagedWhatsAppDetail,
  },
  oauth_connection: {
    Connect: ManagedOAuthConnect,
    Detail: ManagedOAuthDetail,
  },
};
