import type { AppContext } from "./appContext";
import type { ConfirmOptions } from "./confirmDialog";
import { dialogQueue } from "./dialogQueue.svelte";

/** What a prompt dialog shows: the confirm options plus the text field's configuration. */
export interface PromptOptions extends ConfirmOptions {
  /** The text field's starting value; selected on open. */
  initialValue?: string;
  /** The text field's placeholder. */
  placeholder?: string;
  /** Returns an already-translated error message for an invalid value, or `null` when valid. A
   * non-null result disables submission and is shown under the field. */
  validate?: (value: string) => string | null;
}

/**
 * Shows a modal single-line prompt dialog and resolves with the entered text. Escape and the
 * Cancel button both resolve `null`; a click on the backdrop does nothing. The dialog focuses and
 * selects the text field, submits on Enter, and traps Tab inside itself.
 * @param ctx - Supplies the translator `t`.
 * @param opts - The dialog's text keys, styling and field configuration.
 * @returns The entered text, or `null` when dismissed.
 * @example
 * const name = await promptDialog(ctx, { titleKey: "x.title", messageKey: "x.message", confirmLabelKey: "x.ok", cancelLabelKey: "x.cancel" });
 */
export function promptDialog(ctx: Pick<AppContext, "t">, opts: PromptOptions): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    dialogQueue.open((id) => ({
      id,
      kind: "prompt",
      title: ctx.t(opts.titleKey),
      message: ctx.t(opts.messageKey, opts.messageParams),
      confirmLabel: ctx.t(opts.confirmLabelKey),
      cancelLabel: ctx.t(opts.cancelLabelKey),
      danger: opts.danger ?? false,
      initialValue: opts.initialValue ?? "",
      placeholder: opts.placeholder ?? "",
      validate: opts.validate,
      settle: (value) => {
        dialogQueue.close(id);
        resolve(value);
      },
    }));
  });
}
