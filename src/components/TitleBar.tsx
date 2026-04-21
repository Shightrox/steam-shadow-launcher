import { api } from "../api/tauri";
import { useI18n } from "../i18n";

export function TitleBar() {
  const { t } = useI18n();
  const onMinimize = () => api.minimizeWindow();
  const onClose = () => api.closeWindow();
  // Use Tauri's data attribute so the OS handles drag without flicker.
  return (
    <div className="titlebar">
      <div className="tb-drag" data-tauri-drag-region>
        <span className="tb-logo" data-tauri-drag-region>▓ {t("app.name")}</span>
        <span className="tb-sub" data-tauri-drag-region>{t("app.subtitle")}</span>
      </div>
      <button
        className="tb-btn"
        title={t("tb.min")}
        onClick={onMinimize}
        aria-label="minimize"
      >
        ─
      </button>
      <button
        className="tb-btn tb-close"
        title={t("tb.close")}
        onClick={onClose}
        aria-label="close"
      >
        ×
      </button>
    </div>
  );
}
