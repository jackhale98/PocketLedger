import { useEffect, type RefObject } from "react";
import { canGoBack, goBack } from "../store/backStore";

/** A swipe has to travel this far, stay mostly horizontal, and finish briskly,
 *  so that scrolling a list or tapping a row is never mistaken for one. */
const MIN_DISTANCE = 60;
const HORIZONTAL_RATIO = 1.5;
const MAX_DURATION_MS = 800;

/** True when the touch began inside something that scrolls sideways itself --
 *  the period table, a wide register row. Those must keep their own gesture. */
function insideHorizontalScroller(start: Element | null, root: Element): boolean {
  let el: Element | null = start;
  while (el && el !== root) {
    const style = window.getComputedStyle(el);
    const scrolls = style.overflowX === "auto" || style.overflowX === "scroll";
    if (scrolls && el.scrollWidth > el.clientWidth + 1) return true;
    el = el.parentElement;
  }
  return false;
}

/** True for controls a horizontal drag means something to. */
function isDraggableControl(target: EventTarget | null): boolean {
  const el = target instanceof Element ? target.closest("input,textarea,select") : null;
  if (!el) return false;
  return !(el instanceof HTMLInputElement) || el.type === "range" || el.type === "text";
}

/**
 * Swipe left and right over the page body.
 *
 * Where something is open that back would close, a rightward swipe closes it
 * and a leftward one does nothing -- paging to another tab from inside a
 * drill-down would throw away the place the user was looking at. On a plain
 * tab screen the two directions step between tabs instead.
 */
export function useSwipeNavigation(
  ref: RefObject<HTMLElement | null>,
  tabs: readonly string[],
  activeTab: string,
  onTabChange: (tab: string) => void
): void {
  useEffect(() => {
    const root = ref.current;
    if (!root) return;

    let startX = 0;
    let startY = 0;
    let startedAt = 0;
    let armed = false;

    const onTouchStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) {
        armed = false;
        return;
      }
      const touch = e.touches[0];
      const target = touch.target instanceof Element ? touch.target : null;
      armed =
        !insideHorizontalScroller(target, root) && !isDraggableControl(touch.target);
      startX = touch.clientX;
      startY = touch.clientY;
      startedAt = e.timeStamp;
    };

    const onTouchEnd = (e: TouchEvent) => {
      if (!armed) return;
      armed = false;
      const touch = e.changedTouches[0];
      if (!touch) return;

      const dx = touch.clientX - startX;
      const dy = touch.clientY - startY;
      if (e.timeStamp - startedAt > MAX_DURATION_MS) return;
      if (Math.abs(dx) < MIN_DISTANCE) return;
      if (Math.abs(dx) < Math.abs(dy) * HORIZONTAL_RATIO) return;

      if (canGoBack()) {
        if (dx > 0) goBack();
        return;
      }

      const index = tabs.indexOf(activeTab);
      if (index === -1) return;
      const next = dx > 0 ? index - 1 : index + 1;
      if (next >= 0 && next < tabs.length) onTabChange(tabs[next]);
    };

    // Passive: the gesture never calls preventDefault, so scrolling stays
    // smooth and the browser is free to ignore us mid-scroll.
    root.addEventListener("touchstart", onTouchStart, { passive: true });
    root.addEventListener("touchend", onTouchEnd, { passive: true });
    return () => {
      root.removeEventListener("touchstart", onTouchStart);
      root.removeEventListener("touchend", onTouchEnd);
    };
  }, [ref, tabs, activeTab, onTabChange]);
}
