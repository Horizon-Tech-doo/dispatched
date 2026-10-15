import { observeElementRect, type Rect, type Virtualizer } from "@tanstack/react-virtual";

/**
 * `observeElementRect` that keeps `initialRect` as the floor: virtual-core overwrites `initialRect`
 * with the first measurement, and an unlaid-out element (jsdom, a hidden pane) measures 0x0.
 */
export function observeRectWithFloor(initialRect?: Rect) {
  return <T extends Element>(instance: Virtualizer<T, Element>, cb: (rect: Rect) => void) =>
    observeElementRect(instance, (rect) => cb(initialRect && rect.height === 0 && rect.width === 0 ? initialRect : rect));
}
