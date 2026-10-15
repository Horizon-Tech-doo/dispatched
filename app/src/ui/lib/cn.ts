import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// Teach tailwind-merge our custom font-size scale (so `text-12` and `text-muted` don't collide)
// the `row` spacing step (so `h-row` and `h-4` dedupe), and our radius and shadow tokens.
const merge = extendTailwindMerge({
  extend: {
    theme: { spacing: ["row"], radius: ["ctl"], shadow: ["float"] },
    classGroups: { "font-size": [{ text: ["11", "12", "13", "14"] }] },
  },
});

export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
