/** Fields shared by every queued dialog request. Text is already translated by the caller's `t`. */
interface DialogRequestBase {
  /** Unique id keying the request in the host's each-block. */
  id: number;
  /** The dialog heading. */
  title: string;
  /** The body text. */
  message: string;
  /** The primary button's label. */
  confirmLabel: string;
  /** The cancel button's label. */
  cancelLabel: string;
  /** Styles the primary button as destructive and marks the dialog an alert dialog. */
  danger: boolean;
}

/** A queued confirm request; `settle` resolves the caller's promise. */
export interface ConfirmRequest extends DialogRequestBase {
  /** Discriminant selecting `ConfirmDialog`. */
  kind: "confirm";
  /** Resolves the promise with `true` (confirmed) or `false` (Escape or Cancel). */
  settle: (confirmed: boolean) => void;
}

/** A queued prompt request; `settle` resolves the caller's promise. */
export interface PromptRequest extends DialogRequestBase {
  /** Discriminant selecting `PromptDialog`. */
  kind: "prompt";
  /** The text input's starting value. */
  initialValue: string;
  /** The text input's placeholder; empty for none. */
  placeholder: string;
  /** Returns an error message for an invalid value, or `null` when valid. */
  validate: ((value: string) => string | null) | undefined;
  /** Resolves the promise with the entered text, or `null` for Escape or Cancel. */
  settle: (value: string | null) => void;
}

/** Any queued dialog request. */
export type DialogRequest = ConfirmRequest | PromptRequest;

/**
 * The reactive list of open dialogs that `DialogHost` renders, oldest first. Module-level so
 * `confirmDialog` and `promptDialog` need no AppContext handle to open one.
 */
class DialogQueue {
  /** The open dialogs, oldest first. */
  items = $state<DialogRequest[]>([]);
  /** Monotonic id source for `DialogRequestBase.id`. */
  #nextId = 1;

  /**
   * Opens a dialog by appending its request.
   * @param build - Receives the new request id and returns the full request.
   * @returns Nothing; the request is rendered by `DialogHost`.
   * @example
   * dialogQueue.open((id) => ({ id, kind: "confirm", title: "", message: "", confirmLabel: "OK", cancelLabel: "Cancel", danger: false, settle: () => {} }));
   */
  open(build: (id: number) => DialogRequest): void {
    this.items.push(build(this.#nextId++));
  }

  /**
   * Removes a request after it settles.
   * @param id - The `DialogRequestBase.id` to drop; an unknown id is ignored.
   * @example
   * dialogQueue.close(1);
   */
  close(id: number): void {
    this.items = this.items.filter((r) => r.id !== id);
  }
}

/** The shared dialog queue. */
export const dialogQueue = new DialogQueue();
