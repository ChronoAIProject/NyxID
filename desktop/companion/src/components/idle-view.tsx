import {
  BellOff,
  MessageCircle,
  Settings,
  UtensilsCrossed,
} from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent } from "react";

import type { WindowDragEvent } from "../runtime";

import { IconButton } from "./icon-button";
import { Mascot, type MascotState } from "./mascot";

const DRAG_THRESHOLD_PX = 4;
const LOOK_DEADZONE = 0.16;
const PET_REACTION_MS = 1_100;
const PET_REACTIONS = ["我在呢", "收到一个摸摸", "陪着你"] as const;

type PointerInteraction =
  "hover" | "pressed" | "petted" | "dragging-left" | "dragging-right";

interface LookVector {
  readonly x: number;
  readonly y: number;
}

const NEUTRAL_LOOK: LookVector = { x: 0, y: 0 };

function clampUnit(value: number): number {
  return Math.max(-1, Math.min(1, value));
}

interface IdleViewProps {
  readonly name: string;
  readonly nextMeal: string;
  readonly quiet: boolean;
  readonly mascotState?: MascotState;
  readonly dragEvent?: WindowDragEvent;
  readonly onOpen: () => void;
  readonly onMeal: () => void;
  readonly onSettings: () => void;
  readonly onStartDrag: () => Promise<void>;
  readonly onToggleQuiet: () => void;
}

function releasePointerCapture(
  target: HTMLButtonElement,
  pointerId: number,
): void {
  if (
    typeof target.hasPointerCapture === "function" &&
    target.hasPointerCapture(pointerId)
  ) {
    target.releasePointerCapture(pointerId);
  }
}

export function IdleView({
  name,
  nextMeal,
  quiet,
  mascotState = "idle",
  dragEvent,
  onOpen,
  onMeal,
  onSettings,
  onStartDrag,
  onToggleQuiet,
}: IdleViewProps) {
  const pointerStart = useRef<{
    x: number;
    y: number;
    pointerId: number;
  }>();
  const dragged = useRef(false);
  const suppressClick = useRef(false);
  const hovered = useRef(false);
  const petReactionIndex = useRef(0);
  const petReactionTimer = useRef<number>();
  const mascotButton = useRef<HTMLButtonElement>(null);
  const [interaction, setInteraction] = useState<PointerInteraction>();
  const [look, setLook] = useState<LookVector>(NEUTRAL_LOOK);
  const [petReaction, setPetReaction] = useState<string>();
  const nativeInteraction: PointerInteraction | undefined =
    dragEvent?.phase === "moving"
      ? dragEvent.direction === "left"
        ? "dragging-left"
        : "dragging-right"
      : undefined;
  const visibleInteraction = nativeInteraction ?? interaction;

  const visibleMascotState: MascotState =
    visibleInteraction === "dragging-left"
      ? "running-left"
      : visibleInteraction === "dragging-right"
        ? "running-right"
        : visibleInteraction === "pressed"
          ? "waving"
          : visibleInteraction === "petted"
            ? "happy"
            : mascotState;

  const visibleLook = visibleInteraction === "hover" ? look : NEUTRAL_LOOK;

  function clearPetReactionTimer() {
    if (petReactionTimer.current === undefined) return;
    window.clearTimeout(petReactionTimer.current);
    petReactionTimer.current = undefined;
  }

  function updateLook(event: PointerEvent<HTMLButtonElement>) {
    const bounds = event.currentTarget.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) {
      setLook(NEUTRAL_LOOK);
      return;
    }

    const x = clampUnit(
      (event.clientX - (bounds.left + bounds.width / 2)) / (bounds.width / 2),
    );
    const y = clampUnit(
      (event.clientY - (bounds.top + bounds.height / 2)) / (bounds.height / 2),
    );
    setLook(Math.hypot(x, y) < LOOK_DEADZONE ? NEUTRAL_LOOK : { x, y });
  }

  function reactToPet() {
    clearPetReactionTimer();
    const message = PET_REACTIONS[petReactionIndex.current];
    petReactionIndex.current =
      (petReactionIndex.current + 1) % PET_REACTIONS.length;
    setLook(NEUTRAL_LOOK);
    setPetReaction(message);
    setInteraction("petted");
    petReactionTimer.current = window.setTimeout(() => {
      petReactionTimer.current = undefined;
      setPetReaction(undefined);
      setInteraction(hovered.current ? "hover" : undefined);
    }, PET_REACTION_MS);
  }

  function settleInteraction() {
    setInteraction(hovered.current ? "hover" : undefined);
  }

  useEffect(() => {
    if (dragEvent?.phase !== "settled") return;
    const pointer = pointerStart.current;
    if (pointer && mascotButton.current) {
      releasePointerCapture(mascotButton.current, pointer.pointerId);
    }
    pointerStart.current = undefined;
    setInteraction(hovered.current ? "hover" : undefined);
  }, [dragEvent]);

  useEffect(
    () => () => {
      if (petReactionTimer.current === undefined) return;
      window.clearTimeout(petReactionTimer.current);
      petReactionTimer.current = undefined;
    },
    [],
  );

  function beginPointer(event: PointerEvent<HTMLButtonElement>) {
    if (event.button !== 0 || !event.isPrimary) return;
    clearPetReactionTimer();
    setPetReaction(undefined);
    setLook(NEUTRAL_LOOK);
    pointerStart.current = {
      x: event.screenX,
      y: event.screenY,
      pointerId: event.pointerId,
    };
    dragged.current = false;
    suppressClick.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
    setInteraction("pressed");
  }

  function movePointer(event: PointerEvent<HTMLButtonElement>) {
    const start = pointerStart.current;
    if (!event.isPrimary) return;
    if (!start) {
      updateLook(event);
      return;
    }
    if (start.pointerId !== event.pointerId) return;

    const deltaX = event.screenX - start.x;
    const deltaY = event.screenY - start.y;
    if (dragged.current) {
      setInteraction(deltaX < 0 ? "dragging-left" : "dragging-right");
      return;
    }
    if (Math.hypot(deltaX, deltaY) < DRAG_THRESHOLD_PX) return;

    dragged.current = true;
    suppressClick.current = true;
    setInteraction(deltaX < 0 ? "dragging-left" : "dragging-right");

    const pointerId = event.pointerId;
    const target = event.currentTarget;
    void onStartDrag().catch(() => {
      if (pointerStart.current?.pointerId === pointerId) {
        pointerStart.current = undefined;
        releasePointerCapture(target, pointerId);
        settleInteraction();
      }
    });
  }

  function endPointer(event: PointerEvent<HTMLButtonElement>) {
    if (pointerStart.current?.pointerId !== event.pointerId) return;
    releasePointerCapture(event.currentTarget, event.pointerId);
    pointerStart.current = undefined;
    settleInteraction();
  }

  return (
    <main className="idle-view">
      <div className="idle-bubble" aria-live="polite">
        <strong>{name}</strong>
        <span>{petReaction ?? (quiet ? "饭点提醒已暂停" : nextMeal)}</span>
      </div>

      <button
        ref={mascotButton}
        type="button"
        className={`mascot-button${visibleInteraction?.startsWith("dragging") ? " is-dragging" : ""}`}
        data-interaction={visibleInteraction ?? "resting"}
        onPointerDown={beginPointer}
        onPointerMove={movePointer}
        onPointerUp={endPointer}
        onPointerCancel={endPointer}
        onPointerEnter={() => {
          hovered.current = true;
          if (!pointerStart.current && petReactionTimer.current === undefined) {
            setInteraction("hover");
          }
        }}
        onPointerLeave={() => {
          hovered.current = false;
          setLook(NEUTRAL_LOOK);
          if (!pointerStart.current && petReactionTimer.current === undefined) {
            setInteraction(undefined);
          }
        }}
        onClick={(event) => {
          if (suppressClick.current) {
            event.preventDefault();
            event.stopPropagation();
            suppressClick.current = false;
            dragged.current = false;
            return;
          }
          dragged.current = false;
          reactToPet();
        }}
        aria-label={`摸摸 ${name}；拖动可以移动`}
      >
        <Mascot state={visibleMascotState} size={176} look={visibleLook} />
      </button>

      <div className="idle-actions">
        <IconButton label="和 NyxID 对话" onClick={onOpen}>
          <MessageCircle aria-hidden="true" />
        </IconButton>
        <IconButton label="现在选吃的" onClick={onMeal}>
          <UtensilsCrossed aria-hidden="true" />
        </IconButton>
        <IconButton
          label={quiet ? "恢复饭点提醒" : "暂停饭点提醒"}
          tone={quiet ? "active" : "default"}
          onClick={onToggleQuiet}
        >
          <BellOff aria-hidden="true" />
        </IconButton>
        <IconButton label="设置" onClick={onSettings}>
          <Settings aria-hidden="true" />
        </IconButton>
      </div>
    </main>
  );
}
