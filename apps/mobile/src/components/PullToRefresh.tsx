import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

type Phase = "idle" | "pulling" | "ready" | "refreshing" | "done";

/** Drag distance (px) at which releasing triggers a refresh. */
const THRESHOLD = 64;
/** Hard cap so a long drag never pushes the content off screen. */
const MAX_PULL = 108;
/** Extra drag beyond the threshold is damped so the indicator "resists". */
const RESISTANCE = 0.5;
/** Where the indicator rests while a refresh is in flight or just finished. */
const HOLD = 48;
/** The determinate ring's geometry (viewBox is 28×28). */
const RING_RADIUS = 10;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

const LABELS: Record<Phase, string> = {
  idle: "Pull to refresh",
  pulling: "Pull to refresh",
  ready: "Release to refresh",
  refreshing: "Refreshing…",
  done: "Updated",
};

/**
 * Pull-to-refresh for the app's scroll container. The gesture is only armed at
 * the very top of the list, so it never fights normal scrolling.
 *
 * The indicator follows the standard progression (Apple activity indicator /
 * Material 3 loading indicator):
 *   pull  → a determinate progress ring fills with the drag distance,
 *   ready → the chevron flips and the label says "Release to refresh",
 *   refresh → the ring becomes an indeterminate spinner while data loads,
 *   done  → a check mark confirms the reload before the indicator resets.
 *
 * Every phase is announced through an `aria-live` region, so the reload is
 * never silent for screen-reader users either.
 */
export function PullToRefresh({
  onRefresh,
  children,
  disabled = false,
}: {
  onRefresh: () => Promise<void>;
  children: ReactNode;
  disabled?: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const startY = useRef<number | null>(null);
  const phaseRef = useRef<Phase>("idle");
  const mounted = useRef(true);
  const resetTimer = useRef<number | null>(null);

  const [pull, setPull] = useState(0);
  const [phase, setPhase] = useState<Phase>("idle");
  const [dragging, setDragging] = useState(false);

  const setPhaseBoth = useCallback((next: Phase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (resetTimer.current != null) window.clearTimeout(resetTimer.current);
    };
  }, []);

  const finish = useCallback(async () => {
    setPhaseBoth("refreshing");
    setPull(HOLD);
    setDragging(false);
    try {
      await onRefresh();
    } catch {
      // A failed refresh is surfaced by the screen's own error state; the
      // indicator must still return to idle so the gesture stays usable.
    }
    if (!mounted.current) return;
    setPhaseBoth("done");
    setPull(HOLD);
    resetTimer.current = window.setTimeout(() => {
      if (mounted.current) {
        setPull(0);
        setPhaseBoth("idle");
      }
    }, 750);
  }, [onRefresh, setPhaseBoth]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return undefined;

    const onStart = (event: TouchEvent) => {
      if (disabled || phaseRef.current === "refreshing") return;
      if (el.scrollTop > 0 || event.touches.length !== 1) return;
      startY.current = event.touches[0].clientY;
      setDragging(true);
    };

    const onMove = (event: TouchEvent) => {
      if (startY.current == null) return;
      if (el.scrollTop > 0) {
        startY.current = null;
        setPull(0);
        setDragging(false);
        if (phaseRef.current !== "refreshing") setPhaseBoth("idle");
        return;
      }
      const delta = event.touches[0].clientY - startY.current;
      if (delta <= 0) {
        setPull(0);
        if (phaseRef.current !== "refreshing") setPhaseBoth("idle");
        return;
      }
      const damped = Math.min(MAX_PULL, delta * RESISTANCE);
      setPull(damped);
      setPhaseBoth(damped >= THRESHOLD ? "ready" : "pulling");
      // Keep the browser's own overscroll from fighting the gesture.
      if (event.cancelable) event.preventDefault();
    };

    const onEnd = () => {
      if (startY.current == null) return;
      startY.current = null;
      if (phaseRef.current === "ready" && !disabled) {
        void finish();
      } else if (phaseRef.current !== "refreshing") {
        setPull(0);
        setDragging(false);
        setPhaseBoth("idle");
      }
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd);
    el.addEventListener("touchcancel", onEnd);
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, [disabled, finish, setPhaseBoth]);

  const held = phase === "refreshing" || phase === "done";
  const displayPull = held ? HOLD : pull;
  const progress = Math.min(1, pull / THRESHOLD);
  const opacity = phase === "idle" ? 0 : Math.min(1, displayPull / 26);

  return (
    <div className="ptr" ref={containerRef}>
      <div
        className={`ptr__indicator ptr__indicator--${phase}`}
        role="status"
        aria-live="polite"
        style={{
          transform: `translateY(${displayPull - 44}px)`,
          opacity,
        }}
      >
        <span className={`ptr__icon ptr__icon--${phase}`} aria-hidden="true">
          <svg viewBox="0 0 28 28" width="28" height="28" focusable="false">
            <circle className="ptr__track" cx="14" cy="14" r={RING_RADIUS} />
            {phase === "refreshing" ? (
              <circle
                className="ptr__ring ptr__ring--spin"
                cx="14"
                cy="14"
                r={RING_RADIUS}
              />
            ) : (
              <circle
                className="ptr__ring"
                cx="14"
                cy="14"
                r={RING_RADIUS}
                strokeDasharray={RING_CIRCUMFERENCE}
                strokeDashoffset={RING_CIRCUMFERENCE * (1 - progress)}
                transform="rotate(-90 14 14)"
              />
            )}
            {phase === "done" ? (
              <path className="ptr__check" d="M9 14.5l3.2 3.2L19 11" />
            ) : (
              <path className="ptr__arrow" d="M14 18.5V9.5M9.5 14L14 9.5l4.5 4.5" />
            )}
          </svg>
        </span>
        <span className="ptr__label">{LABELS[phase]}</span>
      </div>
      <div
        className="ptr__content"
        style={{
          transform: `translateY(${displayPull}px)`,
          transition: dragging ? "none" : "transform 0.3s cubic-bezier(0.22, 1, 0.36, 1)",
        }}
      >
        {children}
      </div>
    </div>
  );
}
