import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  oldPath: string;
  newPath: string;
  onChoose(strategy: "Move" | "Relink" | "Cancel"): void;
}

export function WorkspaceChangeDialog({ open, oldPath, newPath, onChoose }: Props) {
  const { t } = useI18n();
  if (!open) return null;
  return (
    <div className="modal-overlay" onClick={() => onChoose("Cancel")}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("wsChange.title")}</h2>
        <div className="row">
          <label>{t("wsChange.from")}</label>
          <div className="path">{oldPath}</div>
        </div>
        <div className="row">
          <label>{t("wsChange.to")}</label>
          <div className="path">{newPath}</div>
        </div>
        <p style={{ color: "var(--fg-dim)", fontSize: 10 }}>{t("wsChange.q")}</p>
        <div className="buttons">
          <button onClick={() => onChoose("Cancel")}>{t("common.cancel")}</button>
          <button onClick={() => onChoose("Relink")}>{t("wsChange.relink")}</button>
          <button className="primary" onClick={() => onChoose("Move")}>
            {t("wsChange.move")}
          </button>
        </div>
      </div>
    </div>
  );
}
