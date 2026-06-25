/** Collapse a per-cell A* route into its turn-point vertices: the start, every direction
 * change, and the goal. Each returned segment `runs[i]→runs[i+1]` is a single straight run
 * of collinear clear unit steps, so it crosses no `blocksMove` wall (transitivity along a
 * line) and is gate-valid as one position-update intent. */
export function collinearRuns(path: [number, number][]): [number, number][] {
  if (path.length < 3) return path.map((p) => [p[0], p[1]]);
  const out: [number, number][] = [[path[0][0], path[0][1]]];
  for (let i = 1; i < path.length - 1; i++) {
    const a: [number, number] = [path[i][0] - path[i - 1][0], path[i][1] - path[i - 1][1]];
    const b: [number, number] = [path[i + 1][0] - path[i][0], path[i + 1][1] - path[i][1]];
    const cross = a[0] * b[1] - a[1] * b[0];
    const dot = a[0] * b[0] + a[1] * b[1];
    const scale = Math.max(Math.hypot(...a) * Math.hypot(...b), 1e-9);
    // A turn = non-collinear (cross != 0) or a reversal (dot < 0). Keep this vertex.
    if (Math.abs(cross) > 1e-6 * scale || dot < 0) out.push([path[i][0], path[i][1]]);
  }
  out.push([path[path.length - 1][0], path[path.length - 1][1]]);
  return out;
}
