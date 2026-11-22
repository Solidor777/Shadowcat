import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
import { DUCKING_MODULE_ID, type KeySourceBindings } from "../keySource";

/**
 * Builds a real `KeybindingsController` over a fresh hook bus, plus the `ctx.keybindings` slice
 * a `KeySource` receives, stamped with the ducking module id exactly as `ModuleRegistry` does.
 * @param target Where the controller listens for keydown; defaults to `window` when one exists.
 * @returns The controller (for rebinding and disposal) and the module-scoped slice.
 * @example
 * ```
 * const { controller, bindings } = makeBindings();
 * controller.dispose();
 * ```
 */
export function makeBindings(target?: EventTarget): {
  /** The controller backing the slice. */
  controller: KeybindingsController;
  /** The `ctx.keybindings` slice a `KeySource` registers through. */
  bindings: KeySourceBindings;
} {
  const controller = new KeybindingsController(new HookBus(silentLogger), target);
  return {
    controller,
    bindings: {
      register: (bindingId, opts) => controller.register(DUCKING_MODULE_ID, bindingId, opts),
      bindingsFor: (moduleId) => controller.bindingsFor(moduleId),
    },
  };
}
