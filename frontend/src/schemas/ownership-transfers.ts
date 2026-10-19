import { z } from "zod";

export const ownershipTransferSchema = z.object({
  new_owner_user_id: z.uuid("Select a destination person or organization"),
});

export type OwnershipTransferForm = z.infer<typeof ownershipTransferSchema>;
