import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  onCancel(): void;
  onContinue(): void;
}

/** Shown before triggering UAC for sandbox-mode relaunch, so the user is not
 *  surprised by the admin prompt. */
export function AdminRestartDialog({ open, onCancel, onContinue }: Props) {
  const { t } = useI18n();
  if (!open) return null;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ maxWidth: 420 }}>
        <h2>:: {t("admin.restartTitle")}</h2>
        <p style={{ fontSize: 11, color: "var(--fg-dim)", lineHeight: 1.5 }}>
          {t("admin.restartBody")}
        </p>
        <div className="buttons">
          <button className="ghost" onClick={onCancel}>
            {t("admin.cancel")}
          </button>
          <button className="primary" onClick={onContinue}>
            {t("admin.continue")}
          </button>
        </div>
      </div>
    </div>
  );
}
