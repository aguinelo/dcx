/**
 * Shared type for live background runs. The renderer receives their data
 * from dcode events; no mock terminal output is kept here.
 */

/**
 * A long-running task that outlives the turn that started it.
 *
 * The reference shows these in their own panel block with a live elapsed
 * counter and a switch that stops them, which is what makes them read as
 * background work rather than more chat.
 */
export interface BackgroundTask {
  id: string;
  glyph: string;
  title: string;
  /** Seconds already on the clock when the panel first renders. */
  elapsed: number;
  running: boolean;
}
