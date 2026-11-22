import type { AppContext } from "./appContext";
import { dialogQueue } from "./dialogQueue.svelte";

/** What a confirm dialog shows. Every `*Key` is an i18n key resolved through the caller's `t`. */
export interface ConfirmOptions {
  /** Heading i18n key. */
  titleKey: string;
  /** Body i18n key. */
  messageKey: string;
  /** Interpolation values for the body message. */
  messageParams?: Record<string, string | number>;
  /** Primary button label i18n key. */
  confirmLabelKey: string;
  /** Cancel button label i18n key. */
  cancelLabelKey: string;
  /** Marks the action destructive: danger styling and an alert-dialog role. */
  danger?: boolean;
}

/**
 * Shows a modal confirm dialog and resolves with the user's choice. Escape and the Cancel button
 * both resolve `false`; a click on the backdrop does nothing. The dialog focuses its primary
 * button, submits on Enter, and traps Tab inside itself.
 * @param ctx - Supplies the translator `t`.
 * @param opts - The dialog's text keys and styling.
 * @returns `true` when confirmed, `false` when dismissed.
 * @example
 * if (await confirmDialog(ctx, { titleKey: "x.title", messageKey: "x.message", confirmLabelKey: "x.ok", cancelLabelKey: "x.cancel", danger: true })) remove();
 */
export function confirmDialog(ctx: Pick<AppContext, "t">, opts: ConfirmOptions): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    dialogQueue.open((id) => ({
      id,
      kind: "confirm",
      title: ctx.t(opts.titleKey),
      message: ctx.t(opts.messageKey, opts.messageParams),
      confirmLabel: ctx.t(opts.confirmLabelKey),
      cancelLabel: ctx.t(opts.cancelLabelKey),
      danger: opts.danger ?? false,
      settle: (confirmed) => {
        dialogQueue.close(id);
        resolve(confirmed);
      },
    }));
  });
}
