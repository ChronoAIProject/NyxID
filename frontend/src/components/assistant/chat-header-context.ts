import { createContext } from "react";

export const ChatHeaderCompactContext = createContext(false);
/** False when the latest compact change happened at the transcript's end
 * (a chat opened at, or following, its latest message): the header then
 * switches in place instead of gliding. */
export const ChatHeaderAnimateContext = createContext(true);
/**
 * Where the chat width grips render: the shell's unclipped layer over the chat
 * column, so the grip tabs can stand above the rounded content slab. Null
 * renders them in place.
 */
export const ChatGripSlotContext = createContext<HTMLElement | null>(null);
