/** The tree never gets narrower than this many pixels, so a row keeps room for its counts beside a name. */
export const TREE_MIN_PX = 200;

/** The tree's smallest share (percent) in a window `width` px wide: the pixel minimum, within 12 to 45 percent. */
export const treeMinPercent = (width: number) => Math.min(45, Math.max(12, Math.ceil((TREE_MIN_PX / Math.max(width, 1)) * 100)));

/** What the tree shows: the saved size, raised to the minimum when the window is narrow. */
export const treeShownSize = (saved: number, min: number) => Math.max(saved, min);
