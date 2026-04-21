import { useEffect, useState } from "react";
import { useApp } from "./state/store";
import { FirstRunWizard } from "./views/FirstRunWizard";
import { MainView } from "./views/MainView";
import { SettingsView } from "./views/SettingsView";
import { AuthenticatorView } from "./views/AuthenticatorView";
import { LogDrawer } from "./components/LogDrawer";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { ToastHost } from "./components/ToastHost";
import { useI18n, type Lang } from "./i18n";
import { listen } from "@tauri-apps/api/event";
import {
  AUTH_AUTO_CONFIRMED_EVENT,
  AUTH_CONFIRMS_EVENT,
  type Confirmation,
} from "./api/tauri";

export type Route = "main" | "settings" | "auth";

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
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      // Allow text selection inside destructive-confirm dialogs so the user
      // can copy the literal type-to-confirm phrase.
      const target = e.target as HTMLElement | null;
      if (target && target.closest(".confirm-body")) return;
      e.preventDefault();
    },
    true
  );
}

export default function App() {
  const {
    settings,
    accounts,
    authStatus,
    bootstrap,
    refreshCode,
    mergeConfirmations,
    toast,
  } = useApp();
  const { t, setLang, lang } = useI18n();
  const [route, setRoute] = useState<Route>("main");
  const [logOpen, setLogOpen] = useState(false);

  useEffect(() => {
    installGuards();
    bootstrap();
  }, []);

  // P11 M5: Listen for poller events from the Rust backend.
  useEffect(() => {
    const unlisteners: Array<Promise<() => void>> = [];
    unlisteners.push(
      listen<{ login: string; count: number; items: Confirmation[] }>(
        AUTH_CONFIRMS_EVENT,
        (e) => {
          mergeConfirmations(e.payload.login, e.payload.items);
        },
      ),
    );
    unlisteners.push(
      listen<{ login: string; ids: string[] }>(AUTH_AUTO_CONFIRMED_EVENT, (e) => {
        toast(
          "success",
          t("auth.poller.autoConfirmed", {
            count: e.payload.ids.length,
            login: e.payload.login,
          }),
        );
      }),
    );
    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()));
    };
  }, []);

  useEffect(() => {
    if (settings?.language && settings.language !== lang) {
      setLang(settings.language as Lang);
    }
  }, [settings?.language]);

  // P11: refresh all guard codes every 30 seconds + initial pull.
  useEffect(() => {
    const loginsWithAuth = accounts
      .filter((a) => a.hasAuthenticator || authStatus[a.login]?.hasAuthenticator)
      .map((a) => a.login);
    if (!loginsWithAuth.length) return;
    let cancelled = false;
    const pump = async () => {
      for (const login of loginsWithAuth) {
        if (cancelled) return;
        await refreshCode(login);
      }
    };
    pump();
    const iv = setInterval(pump, 30_000);
    return () => {
      cancelled = true;
      clearInterval(iv);
    };
  }, [accounts, authStatus]);

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
            ) : route === "auth" ? (
              <AuthenticatorView />
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
