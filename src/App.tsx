import { useEffect, useState } from "react";
import { useApp } from "./state/store";
import { FirstRunWizard } from "./views/FirstRunWizard";
import { MainView } from "./views/MainView";
import { SettingsView } from "./views/SettingsView";
import { LogDrawer } from "./components/LogDrawer";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { ToastHost } from "./components/ToastHost";
import { useI18n, type Lang } from "./i18n";

// Block right-click, DevTools hotkeys, reload etc. in production-ish way.
function installGuards() {
  const stop = (e: Event) => {
    e.preventDefault();
  };
  window.addEventListener("contextmenu", stop, true);
  window.addEventListener(
    "keydown",
    (e) => {
      const k = e.key;
      if (k === "F12" || k === "F5") {
        e.preventDefault();
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        if (e.shiftKey && (k === "I" || k === "J" || k === "C" || k === "K")) {
          e.preventDefault();
          return;
        }
        if (k === "R" || k === "r" || k === "U" || k === "u" || k === "P" || k === "p") {
          e.preventDefault();
          return;
        }
      }
    },
    true
  );
  // Block text selection in general, allow only inputs
  document.addEventListener(
    "selectstart",
    (e) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag !== "INPUT" && tag !== "TEXTAREA") e.preventDefault();
    },
    true
  );
}

export default function App() {
  const { settings, bootstrap } = useApp();
  const { t, setLang, lang } = useI18n();
  const [route, setRoute] = useState<"main" | "settings">("main");
  const [logOpen, setLogOpen] = useState(false);

  useEffect(() => {
    installGuards();
    bootstrap();
  }, []);

  useEffect(() => {
    if (settings?.language && settings.language !== lang) {
      setLang(settings.language as Lang);
    }
  }, [settings?.language]);

  if (!settings) {
    return (
      <div className="app">
        <TitleBar />
        <div className="shell">
          <div className="content">
            <div className="empty">{t("common.booting")}</div>
          </div>
        </div>
      </div>
    );
  }

  const showWizard = !settings.firstRunCompleted || !settings.workspace;

  return (
    <div className="app">
      <TitleBar />
      {showWizard ? (
        <div className="shell no-side">
          <div className="content">
            <FirstRunWizard />
          </div>
        </div>
      ) : (
        <div className="shell">
          <Sidebar
            view={route}
            setView={setRoute}
            toggleLog={() => setLogOpen((v) => !v)}
          />
          <div className="content">
            {route === "settings" ? (
              <SettingsView onClose={() => setRoute("main")} />
            ) : (
              <MainView />
            )}
          </div>
        </div>
      )}
      <div className="statusbar">
        <span className="blink">●</span>
        <span>{t("status.ready")}</span>
        <span className="dim">·</span>
        <span>
          {t("status.mode")} {(settings.defaultLaunchMode || "switch").toUpperCase()}
        </span>
        <div className="spacer" />
        <span>v{__APP_VERSION__}</span>
      </div>
      <LogDrawer open={logOpen} onClose={() => setLogOpen(false)} />
      <ToastHost />
    </div>
  );
}
