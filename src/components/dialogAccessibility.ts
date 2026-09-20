/** Shared keyboard and labeling behavior for the existing dialog components. */
export function installDialogAccessibility() {
  const previous = new Map<HTMLElement, HTMLElement | null>();
  let serial = 0;
  let lastFocus: HTMLElement | null = document.activeElement as HTMLElement;
  const dialogs = () => Array.from(document.querySelectorAll<HTMLElement>(".modal-overlay > .modal, .modal-backdrop > .modal"));
  const top = () => { const list = dialogs(); return list[list.length - 1]; };
  const controls = (dialog: HTMLElement) => Array.from(dialog.querySelectorAll<HTMLElement>(
    'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]',
  )).filter(e => e.getClientRects().length > 0);
  const focusDialog = (dialog: HTMLElement) => {
    (dialog.querySelector<HTMLElement>("[autofocus]") ?? controls(dialog)[0] ?? dialog).focus();
  };
  const update = () => {
    const current = dialogs();
    for (const [dialog, opener] of previous) {
      if (!dialog.isConnected) {
        previous.delete(dialog);
        if (opener?.isConnected) opener.focus();
      }
    }
    for (const dialog of current) {
      if (!previous.has(dialog)) {
        previous.set(dialog, lastFocus && !dialog.contains(lastFocus) ? lastFocus : null);
        dialog.setAttribute("role", "dialog");
        dialog.setAttribute("aria-modal", "true");
        dialog.tabIndex = -1;
        const title = dialog.querySelector<HTMLElement>(".modal-title, h2, h3");
        if (title) {
          title.id ||= `dialog-title-${++serial}`;
          dialog.setAttribute("aria-labelledby", title.id);
        }
        if (dialog === current[current.length - 1] && !dialog.contains(document.activeElement)) focusDialog(dialog);
      }
    }
    for (const label of document.querySelectorAll<HTMLLabelElement>("label:not([for])")) {
      if (label.querySelector("input, select, textarea")) continue;
      const field = label.parentElement?.querySelector<HTMLInputElement>("input, select, textarea");
      if (field) { field.id ||= `field-${++serial}`; label.htmlFor = field.id; }
    }
    const active = top();
    if (active && !active.contains(document.activeElement)) focusDialog(active);
  };
  const keydown = (e: KeyboardEvent) => {
    const dialog = top();
    if (!dialog) return;
    if (e.key === "Escape") {
      e.preventDefault(); e.stopImmediatePropagation();
      // Reuse the component's own close handler and busy/critical-step guards.
      dialog.parentElement?.click();
    } else if (e.key === "Tab") {
      const list = controls(dialog);
      const index = list.indexOf(document.activeElement as HTMLElement);
      if (!list.length) { e.preventDefault(); dialog.focus(); }
      else if (index < 0 || (e.shiftKey && index === 0) || (!e.shiftKey && index === list.length - 1)) {
        e.preventDefault(); list[e.shiftKey ? list.length - 1 : 0].focus();
      }
    }
  };
  const focusin = (e: FocusEvent) => {
    const dialog = top();
    const target = e.target as HTMLElement;
    if (dialog && !dialog.contains(target)) { focusDialog(dialog); return; }
    if (!dialog) lastFocus = target;
  };
  const observer = new MutationObserver(update);
  observer.observe(document.body, { childList: true, subtree: true });
  document.addEventListener("keydown", keydown, true);
  document.addEventListener("focusin", focusin);
  update();
  return () => {
    observer.disconnect();
    document.removeEventListener("keydown", keydown, true);
    document.removeEventListener("focusin", focusin);
  };
}
