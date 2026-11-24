import { describe, it, expect, beforeEach } from "vitest";
import { rememberFocus, trapFocus } from "./focusTrap";

/** Builds a container of three buttons, attached to the document, and returns it with them. */
function build(): { root: HTMLElement; a: HTMLButtonElement; b: HTMLButtonElement; c: HTMLButtonElement } {
  const root = document.createElement("div");
  const [a, b, c] = ["a", "b", "c"].map((name) => {
    const btn = document.createElement("button");
    btn.textContent = name;
    root.appendChild(btn);
    return btn;
  }) as [HTMLButtonElement, HTMLButtonElement, HTMLButtonElement];
  document.body.appendChild(root);
  return { root, a, b, c };
}

/** Dispatches a Tab keydown on the focused element and returns the event. */
function tab(from: HTMLElement, shiftKey = false): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true });
  from.dispatchEvent(event);
  return event;
}

describe("trapFocus", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("Tab from the last element wraps to the first", () => {
    const { root, a, c } = build();
    trapFocus(root);
    c.focus();
    const event = tab(c);
    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(a);
  });

  it("Shift+Tab from the first element wraps to the last", () => {
    const { root, a, c } = build();
    trapFocus(root);
    a.focus();
    const event = tab(a, true);
    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(c);
  });

  it("leaves Tab between interior elements to native traversal", () => {
    const { root, b } = build();
    trapFocus(root);
    b.focus();
    expect(tab(b).defaultPrevented).toBe(false);
  });

  it("re-queries on every keypress: a control disabled since the last press leaves the cycle", () => {
    const { root, a, b, c } = build();
    trapFocus(root);
    c.focus();
    tab(c);
    expect(document.activeElement).toBe(a);
    c.disabled = true;
    b.focus();
    // `b` is now the last enabled element, so Tab from it wraps; a cached list would not.
    expect(tab(b).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(a);
  });

  it("cancels Tab when the container has no focusable descendant", () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    trapFocus(root);
    expect(tab(root).defaultPrevented).toBe(true);
  });

  it("releasing the trap returns focus to the element focused when it was installed", () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    const { root, b } = build();
    const release = trapFocus(root);
    b.focus();
    root.remove();
    expect(document.activeElement).toBe(document.body);
    release();
    expect(document.activeElement).toBe(opener);
  });

  it("releasing the trap leaves a deliberate focus move elsewhere alone", () => {
    const opener = document.createElement("button");
    const other = document.createElement("button");
    document.body.append(opener, other);
    opener.focus();
    const { root } = build();
    const release = trapFocus(root);
    other.focus();
    release();
    expect(document.activeElement).toBe(other);
  });

  it("rememberFocus does nothing when the recorded element left the document", () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    const { root } = build();
    const restore = rememberFocus(root);
    opener.remove();
    restore();
    expect(document.activeElement).toBe(document.body);
  });

  it("the returned disposer removes the trap", () => {
    const { root, c } = build();
    const release = trapFocus(root);
    release();
    c.focus();
    expect(tab(c).defaultPrevented).toBe(false);
  });
});
