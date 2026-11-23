import type { TourMeta } from "@shadowcat/core";
import { offerableTours, resolvableSteps, type AnchorResolver, type TourViewer } from "./eligibility";
import type { TourProgress } from "./progress";

/** What the controller needs from the module: persisted done-state and the server switch. Both
 * exist as soon as the module registers. */
export interface TourControllerDeps {
  /** The viewer's done-state. */
  progress: TourProgress;
  /** Whether the server allows tours (`onboarding_enabled`); resolves `true` when it cannot say. */
  serverEnabled: () => Promise<boolean>;
}

/** What the controller reads from the running app. The contribution registry and the viewer are
 * reachable only through `AppContext`, which exists once the UI mounts, so the host component
 * supplies these through `TourController.attach`. */
export interface TourEnvironment {
  /** Every contributed tour, in registration order. */
  tours: () => readonly TourMeta[];
  /** The current viewer; read on every decision, so a role or world change is honoured. */
  viewer: () => TourViewer;
  /** Looks a step's anchor id up in the document. */
  resolve: AnchorResolver;
}

/** A tour in progress. */
export interface ActiveTour {
  /** The running tour. */
  tour: TourMeta;
  /** Index into `tour.steps` of the step on screen. */
  stepIndex: number;
  /** Indexes of the steps whose target was mounted at the last move, in step order; the step
   * counter ("2 of 4") counts these, not the declared steps. */
  shown: number[];
}

/** Decides which tour to offer, runs the tour the user starts, and records how it ended.
 *
 * At most one offer is made per controller (one per session): after the user answers it, further
 * tours wait for a later session or the replay list. That paces topic tours instead of chaining
 * a new offer behind every finished one. State is reactive; the offer and the running tour are
 * what the overlay components render. Until `attach` supplies the environment, nothing is
 * offered and `start` refuses. */
export class TourController {
  /** The tour currently offered through the toast, or `null`. */
  offer = $state<TourMeta | null>(null);
  /** The tour currently running, or `null`. */
  active = $state<ActiveTour | null>(null);
  /** Whether this session already made its one offer. */
  #offered = false;
  /** The running app's registry, viewer and anchor lookup; `null` until the host attaches. */
  #env: TourEnvironment | null = null;

  /** Builds a controller over the module's persisted state and server switch.
   * @param deps Done-state and the server switch.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * const controller = new TourController({ progress, serverEnabled: async () => true });
   * ```
   */
  constructor(private readonly deps: TourControllerDeps) {}

  /** Connects the controller to the running app.
   * @param env The registry, viewer and anchor lookup.
   * @returns A function that disconnects it again; the controller then stops offering.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * const controller = new TourController({ progress, serverEnabled: async () => true });
   * const detach = controller.attach({
   *   tours: () => [],
   *   viewer: () => ({ world: "w1", role: "gm", serverRole: "user" }),
   *   resolve: () => null,
   * });
   * detach();
   * ```
   */
  attach(env: TourEnvironment): () => void {
    this.#env = env;
    return () => {
      if (this.#env === env) this.#env = null;
    };
  }

  /** Offers the first eligible tour, unless this session already offered one, a tour is running,
   * the user switched tours off, or the server disallows them. Safe to call repeatedly.
   * @returns Resolves once the decision is made.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * const controller = new TourController({ progress, serverEnabled: async () => true });
   * await controller.evaluate();
   * ```
   */
  async evaluate(): Promise<void> {
    if (this.#env === null || this.#offered || this.active !== null || this.deps.progress.isDisabled()) return;
    if (!(await this.deps.serverEnabled())) return;
    const env = this.#env;
    if (env === null || this.#offered || this.active !== null) return;
    const [first] = offerableTours(env.tours(), env.viewer(), this.deps.progress, env.resolve);
    if (first === undefined) return;
    this.#offered = true;
    this.offer = first;
  }

  /** Starts `tour` at its first step that can be shown, closing any offer.
   * @param tour The tour to run.
   * @returns `false` when the controller is not attached or no step of the tour can be shown, in
   * which case nothing starts.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * const controller = new TourController({ progress, serverEnabled: async () => true });
   * controller.start({
   *   id: "example.tour",
   *   labelKey: "example.tour.name",
   *   version: 1,
   *   audience: "any",
   *   scope: "global",
   *   trigger: "manual",
   *   steps: [{ target: "example.panel", titleKey: "t", bodyKey: "b", placement: "center" }],
   * });
   * ```
   */
  start(tour: TourMeta): boolean {
    if (this.#env === null) return false;
    const shown = resolvableSteps(tour, this.#env.resolve);
    const [stepIndex] = shown;
    if (stepIndex === undefined) return false;
    this.offer = null;
    this.active = { tour, stepIndex, shown };
    return true;
  }

  /** Starts the offered tour.
   * @example
   * ```ts
   * // wired to the offer's Start button
   * ```
   */
  startOffer(): void {
    if (this.offer !== null) this.start(this.offer);
  }

  /** Closes the offer for this session without recording anything; the tour is offered again in
   * a later session.
   * @example
   * ```ts
   * // wired to the offer's Not now button
   * ```
   */
  dismissOffer(): void {
    this.offer = null;
  }

  /** Switches automatic offers off for this account and closes the offer.
   * @example
   * ```ts
   * // wired to the offer's Don't ask again button
   * ```
   */
  disableForever(): void {
    this.deps.progress.setDisabled();
    this.offer = null;
  }

  /** Moves to the next step whose target is mounted now, or ends the tour after the last one.
   * @example
   * ```ts
   * // wired to the step card's Next button
   * ```
   */
  next(): void {
    const current = this.active;
    if (current === null || this.#env === null) return;
    const shown = resolvableSteps(current.tour, this.#env.resolve);
    const stepIndex = shown.find((index) => index > current.stepIndex);
    if (stepIndex === undefined) {
      this.end();
      return;
    }
    this.active = { tour: current.tour, stepIndex, shown };
  }

  /** Moves to the previous step whose target is mounted now; stays put on the first.
   * @example
   * ```ts
   * // wired to the step card's Back button
   * ```
   */
  back(): void {
    const current = this.active;
    if (current === null || this.#env === null) return;
    const shown = resolvableSteps(current.tour, this.#env.resolve);
    const stepIndex = shown.filter((index) => index < current.stepIndex).pop();
    if (stepIndex === undefined) return;
    this.active = { tour: current.tour, stepIndex, shown };
  }

  /** Ends the running tour, finished or skipped, and records its current version as done so it
   * is not offered again until the version is bumped.
   * @example
   * ```ts
   * // wired to the step card's Skip and Done buttons and to Escape
   * ```
   */
  end(): void {
    const current = this.active;
    if (current === null || this.#env === null) return;
    this.deps.progress.markDone(current.tour, this.#env.viewer().world);
    this.active = null;
  }
}
