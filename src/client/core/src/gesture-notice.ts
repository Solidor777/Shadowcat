/** Gestures that have already raised a notice. Weakly held, so a finished gesture is collectable
 * and the set never grows. Module-level on purpose: every notice source a gesture can reach (a
 * tool's route-failure notice, the host's move-rejection notice) goes through
 * `noticeOncePerGesture`, so one gesture has exactly one notice owner however its requests fail. */
const noticedGestures = new WeakSet<object>();

/** Runs `raise` unless `gesture` has already raised a notice through this function, and records
 * that it has. A request issued with no gesture is its own gesture and always raises.
 *
 * Exists for a multi-token drag: it issues one request per token, and tokens can fail in different
 * ways (one route refused, another move rejected), so per-token notices would stack identical or
 * near-identical toasts for one user action. The first failure of the gesture speaks for it.
 * @param gesture - The identity shared by every request one user gesture issues, or `undefined`.
 * @param raise - Raises the notice.
 * @returns Whether `raise` ran.
 * @example
 * ```
 * const gesture = {};
 * const shown: string[] = [];
 * noticeOncePerGesture(gesture, () => shown.push("first"));
 * noticeOncePerGesture(gesture, () => shown.push("second"));
 * noticeOncePerGesture(undefined, () => shown.push("gestureless"));
 * // shown is ["first", "gestureless"]
 * ```
 */
export function noticeOncePerGesture(gesture: object | undefined, raise: () => void): boolean {
  if (gesture !== undefined) {
    if (noticedGestures.has(gesture)) return false;
    noticedGestures.add(gesture);
  }
  raise();
  return true;
}
