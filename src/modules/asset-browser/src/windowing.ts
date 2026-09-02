// Grid-row virtualization math for the thumbnail grid. A sibling of the chat
// panel's list windowing (that helper is module-private and windows single
// rows; this one windows whole grid rows), with its own tests.

/** The half-open item range `[start, end)` the grid mounts. */
export interface GridWindow {
  /** First mounted item index. */
  start: number;
  /** One past the last mounted item index. */
  end: number;
}

/** Rows kept mounted beyond the viewport on each side. */
export const GRID_OVERSCAN_ROWS = 2;

/**
 * The item window whose grid rows intersect the scrolled viewport, padded by
 * `overscanRows` whole rows each side. Row height is derived from the
 * measured `scrollHeight` (the grid's own current layout), so the window
 * tracks real geometry rather than an assumed tile size. Unusable
 * measurements (a hidden tab reads 0 geometry) yield a bounded leading
 * window rather than everything or nothing.
 * @param scrollTop - The container's current scroll offset.
 * @param clientHeight - The container's viewport height.
 * @param scrollHeight - The container's full scrollable height.
 * @param totalCount - Total items in the grid.
 * @param columns - Items per grid row (≥ 1).
 * @param overscanRows - Rows mounted beyond the viewport each side.
 * @returns The `[start, end)` item range to mount.
 * @example
 * ```ts
 * // 100 items in 4 columns, 40px rows, viewport rows 10..15, overscan 1:
 * computeGridWindow(400, 200, 1000, 100, 4, 1); // { start: 36, end: 64 }
 * ```
 */
export function computeGridWindow(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  totalCount: number,
  columns: number,
  overscanRows: number = GRID_OVERSCAN_ROWS,
): GridWindow {
  if (totalCount <= 0) return { start: 0, end: 0 };
  const cols = Math.max(1, Math.floor(columns));
  const rows = Math.ceil(totalCount / cols);
  if (clientHeight <= 0 || scrollHeight <= 0) {
    return { start: 0, end: Math.min(totalCount, cols * (1 + 2 * overscanRows)) };
  }
  const rowHeight = scrollHeight / rows;
  const firstVisible = Math.floor(scrollTop / rowHeight);
  const lastVisible = Math.ceil((scrollTop + clientHeight) / rowHeight);
  const firstRow = Math.max(0, firstVisible - overscanRows);
  const lastRow = Math.min(rows, lastVisible + overscanRows);
  return { start: firstRow * cols, end: Math.min(totalCount, lastRow * cols) };
}
