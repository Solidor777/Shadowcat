/** What the entry stub last received from `App`, for tests that drive the shell from the entry
 * route. */
export const captured: {
  /** The `onEnterWorld` callback `App` passed to the most recent mount. */
  onEnterWorld?: (id: string) => void;
} = {};
