import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { Spinner } from "./Spinner";

/**
 * Re-usable destructive-confirmation dialog.
 *
 * Two modes (controlled by `requireText`):
 *
 *   - Plain "are you sure?" → primary button is enabled immediately.
 *   - Typed-confirmation: user must type `requireText` exactly. Used for the
 *     scariest actions (delete account with files, remove authenticator)
 *     to prevent muscle-memory clicks.
 *
 * The action button is always `danger`-styled and gets focus only after a
 * 600ms grace period, so an accidental Enter doesn't immediately fire it.
 */
interface Props {
  open: boolean;
  title: string;
  /** Body text or a JSX node. Newlines preserved. */
  body: React.ReactNode;
  /** Visible warning bullet points (optional). */
  bullets?: string[];
  /** If set, user must type this string verbatim to enable the action. */
  requireText?: string;
  confirmLabel: string;
  /** Hint shown above the type-to-confirm input. */
  requireHint?: string;
  cancelLabel?: string;
  busy?: boolean;
  onCancel(): void;
  onConfirm(): void | Promise<void>;
}

export function ConfirmDialog({
  open,
  title,
  body,
  bullets,
  requireText,
  confirmLabel,
  requireHint,
  cancelLabel,
  busy,
  onCancel,
  onConfirm,
}: Props) {
  const { t } = useI18n();
  const [typed, setTyped] = useState("");
  const [graceElapsed, setGraceElapsed] = useState(false);
  const cancelRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) return;
    setTyped("");
    setGraceElapsed(false);
    const tm = window.setTimeout(() => setGraceElapsed(true), 600);
    // Park focus on Cancel — opposite of "press Enter to delete".
    cancelRef.current?.focus();
    return () => window.clearTimeout(tm);
  }, [open]);

  if (!open) return null;

  const matches = !requireText || typed.trim() === requireText.trim();
  const canConfirm = matches && graceElapsed && !busy;

  return (
    <div
      className="modal-backdrop"
      onClick={() => !busy && onCancel()}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !busy) onCancel();
      }}
    >
      <div
        className="modal modal-danger"
        onClick={(e) => e.stopPropagation()}
        style={{ minWidth: 420, maxWidth: 520 }}
      >
        <div className="modal-title" style={{ color: "var(--danger)" }}>
          ⚠ {title}
        </div>
        <div className="modal-body">
          <div className="confirm-body">{body}</div>
          {bullets && bullets.length > 0 && (
            <ul className="confirm-bullets">
              {bullets.map((b, i) => (
                <li key={i}>{b}</li>
              ))}
            </ul>
          )}
          {requireText && (
            <div className="field">
              <label>
                {requireHint ?? t("confirm.typeToConfirm", { text: requireText })}
              </label>
              <input
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                placeholder={requireText}
                autoCapitalize="off"
                autoComplete="off"
                spellCheck={false}
              />
            </div>
          )}
        </div>
        <div className="modal-actions">
          <button
            ref={cancelRef}
            className="primary"
            onClick={onCancel}
            disabled={busy}
          >
            {cancelLabel ?? t("common.cancel")}
          </button>
          <button
            className="xs danger"
            disabled={!canConfirm}
            onClick={() => onConfirm()}
            title={
              !graceElapsed
                ? t("confirm.waitGrace")
                : !matches
                  ? t("confirm.typeMatch")
                  : undefined
            }
          >
            {busy ? <Spinner size="xs" inline /> : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
