import { useApp } from "../state/store";
import { useI18n, type Lang } from "../i18n";
import { api } from "../api/tauri";
import { Icon } from "./Icon";
import type { Route } from "../App";

interface Props {
  view: Route;
  setView(v: Route): void;
  toggleLog(): void;
}

export function Sidebar({ view, setView, toggleLog }: Props) {
  const { t, lang, setLang } = useI18n();
  const settings = useApp((s) => s.settings);
  const sbStatus = useApp((s) => s.sbStatus);
  const confirmations = useApp(s => s.confirmations);
  const confirmationCount = Object.values(confirmations).reduce((sum, items) => sum + items.length, 0);

  const changeLanguage = async (language: Lang) => {
    if (!settings) return;
    try {
      await api.saveSettings({ ...settings, language });
      useApp.setState(s => ({ settings: s.settings ? { ...s.settings, language } : null }));
      setLang(language);
    } catch (e) { useApp.getState().toast("error", String(e)); }
  };

  const sbBadge =
    sbStatus === "ready"
      ? t("sb.sbStatus.ready")
      : sbStatus === "installing"
      ? t("sb.sbStatus.installing")
      : sbStatus === "failed"
      ? t("sb.sbStatus.failed")
      : t("sb.sbStatus.missing");

  return (
    <aside className="sidebar">
      <nav className="sb-tabs" aria-label={t("design.navigation")}>
        <button className={`sb-tab ${view === "main" ? "active" : ""}`} onClick={() => setView("main")}><Icon name="accounts" />{t("nav.accounts")}</button>
        <button className={`sb-tab ${view === "confirmations" ? "active" : ""}`} onClick={() => setView("confirmations")}><Icon name="check" />{t("auth.confirmations")}{confirmationCount > 0 && <span className="sb-badge">{confirmationCount}</span>}</button>
        <button className={`sb-tab ${view === "auth" ? "active" : ""}`} onClick={() => setView("auth")}><Icon name="shield" />Steam Guard</button>
        <button className={`sb-tab ${view === "settings" ? "active" : ""}`} onClick={() => setView("settings")}><Icon name="settings" />{t("sb.settings")}</button>
      </nav>
      <div className="sb-spacer" />
      <div className="sb-section sidebar-bottom">
        <div className={`sb-status sb-status-${sbStatus}`}><span className="state-dot" />{sbBadge}</div>
        <div className="lang-toggle">
          <button
            className={`xs ${lang === "ru" ? "active" : ""}`}
            onClick={() => changeLanguage("ru")}
          >RU</button>
          <button
            className={`xs ${lang === "en" ? "active" : ""}`}
            onClick={() => changeLanguage("en")}
          >EN</button>
        </div>
        <button className="sb-link" onClick={toggleLog}>
          ▤ {t("sb.log")}
        </button>
      </div>
    </aside>
  );
}
