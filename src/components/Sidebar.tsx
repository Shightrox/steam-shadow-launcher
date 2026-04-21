import { useState } from "react";
import { useApp } from "../state/store";
import { useI18n, type Lang } from "../i18n";
import type { LaunchMode } from "../api/tauri";
import { SandboxieInstallModal } from "./SandboxieInstallModal";
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
  const sandboxie = useApp((s) => s.sandboxie);
  const setMode = useApp((s) => s.setDefaultMode);
  const authStatus = useApp((s) => s.authStatus);
  const mode: LaunchMode = settings?.defaultLaunchMode ?? "switch";
  const [installOpen, setInstallOpen] = useState(false);

  const authCount = Object.values(authStatus).filter((a) => a.hasAuthenticator).length;

  const choose = async (m: LaunchMode) => {
    if (m === mode) return;
    if (m === "sandbox" && !sandboxie?.installed) {
      setInstallOpen(true);
      return;
    }
    await setMode(m);
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
      <div className="sb-section">
        <div className="sb-title">{t("sb.mode")}</div>
        <button
          className={`mode-btn ${mode === "switch" ? "active" : ""}`}
          onClick={() => choose("switch")}
          data-tip={t("mode.switch.tip")}
        >
          <span className="dot">{mode === "switch" ? "◉" : "○"}</span>
          {t("sb.switch")}
        </button>
        <button
          className={`mode-btn ${mode === "sandbox" ? "active" : ""}`}
          onClick={() => choose("sandbox")}
          data-tip={t("mode.sandbox.tip")}
        >
          <span className="dot">{mode === "sandbox" ? "◉" : "○"}</span>
          {t("sb.sandbox")}
        </button>
        <div className={`sb-status sb-status-${sbStatus}`}>{sbBadge}</div>
      </div>

      <div className="sb-spacer" />

      <div className="sb-section">
        <div className="sb-tabs">
          <button
            className={`sb-tab ${view === "main" ? "active" : ""}`}
            onClick={() => setView("main")}
          >
            👤 {t("nav.accounts")}
          </button>
          <button
            className={`sb-tab ${view === "auth" ? "active" : ""}`}
            onClick={() => setView("auth")}
            data-tip={t("auth.sidebarTip")}
          >
            🔑 {t("nav.authenticator")}
            {authCount > 0 && <span className="sb-badge">{authCount}</span>}
          </button>
        </div>
        <div className="lang-toggle">
          <button
            className={`xs ${lang === "ru" ? "active" : ""}`}
            onClick={() => setLang("ru" as Lang)}
          >RU</button>
          <button
            className={`xs ${lang === "en" ? "active" : ""}`}
            onClick={() => setLang("en" as Lang)}
          >EN</button>
        </div>
        <button
          className={`sb-link ${view === "settings" ? "active" : ""}`}
          onClick={() => setView(view === "settings" ? "main" : "settings")}
        >
          ⚙ {t("sb.settings")}
        </button>
        <button className="sb-link" onClick={toggleLog}>
          ▤ {t("sb.log")}
        </button>
      </div>
      <SandboxieInstallModal
        open={installOpen}
        onClose={() => setInstallOpen(false)}
        onInstalled={async () => {
          await setMode("sandbox");
        }}
      />
    </aside>
  );
}
