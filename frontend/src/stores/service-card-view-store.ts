import { create } from "zustand";
import type { ServiceViewFilters } from "@/schemas/service-view";

interface ServiceCardView {
  accountId?: string;
  expanded: readonly string[];
  filters?: ServiceViewFilters;
  savedViewId?: string;
}

// Unsaved changes survive navigation only. Account defaults come from /users/me.
export const useServiceCardView = create<ServiceCardView>(() => ({
  expanded: [],
}));
