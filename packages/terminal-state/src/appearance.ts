/** Shared native conversation colors; the helper answers only attributes the renderer owns. */
export const NATIVE_TERMINAL_APPEARANCE = {
  foreground: "#ffffff", background: "#282c34", cursor: "#ffffff",
} as const;
export type Appearance = { foreground: string; background: string; cursor: string };
