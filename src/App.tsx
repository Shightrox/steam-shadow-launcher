import { installDialogAccessibility } from "./components/dialogAccessibility";
import { useEffect, useState } from "react";
import { useApp } from "./state/store";
import { FirstRunWizard } from "./views/FirstRunWizard";
import { MainView } from "./views/MainView";
import { SettingsView } from "./views/SettingsView";
import { AuthenticatorView } from "./views/AuthenticatorView";
import { ConfirmationsView } from "./views/ConfirmationsView";
import { LogDrawer } from "./components/LogDrawer";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { AmbientBackground } from "./components/AmbientBackground";
import { ToastHost } from "./components/ToastHost";
import { UpdateModal } from "./components/UpdateModal";
import { useI18n, type Lang } from "./i18n";
import { listen } from "@tauri-apps/api/event";
import {
  AUTH_AUTO_CONFIRMED_EVENT,
  AUTH_CONFIRMS_ERROR_EVENT,
  AUTH_CONFIRMS_EVENT,
  AUTH_SESSION_STATE_EVENT,
  api,
  type Confirmation,
  type SessionState,
  type UpdateInfo,
} from "./api/tauri";

export type Route = "main" | "settings" | "auth" | "confirmations";

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

}

export default function App() {
  const {
    settings,
    bootError,
    accounts,
    authStatus,
    bootstrap,
    refreshCode,
    mergeConfirmations,
    setSessionState,
    toast,
  } = useApp();
  const { t, setLang, lang } = useI18n();
  const [route, setRoute] = useState<Route>("main");
  const [authLogin, setAuthLogin] = useState<string | null>(null);
  const [confirmationLogin, setConfirmationLogin] = useState<string | null>(null);
  const manageAccess = (login?: string) => { setAuthLogin(login ?? null); setRoute("auth"); };
  const showConfirmations = (login?: string) => { setConfirmationLogin(login ?? null); setRoute("confirmations"); };
  const [logOpen, setLogOpen] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);

  useEffect(() => {
    installGuards();
    bootstrap();
    return installDialogAccessibility();
  }, []);

  // Background update check on startup. We wait a few seconds so we don't
  // race with bootstrap()'s settings load + Steam detection. The check is
  // best-effort: any failure (offline, GitHub down, rate-limit) is silently
  // dropped so the launcher still boots.
  useEffect(() => {
    const t = setTimeout(async () => {
      try {
        const info = await api.checkUpdate();
        if (info.has_update && info.download_url) {
          setUpdateInfo(info);
        }
      } catch {
        /* network errors are non-fatal */
      }
    }, 4000);
    return () => clearTimeout(t);
  }, []);

  // P11 M5: Listen for poller events from the Rust backend.
  useEffect(() => {
    const unlisteners: Array<Promise<() => void>> = [];
    unlisteners.push(
      listen<{ workspace: string; login: string; message: string }>(AUTH_CONFIRMS_ERROR_EVENT, (e) => {
        if (e.payload.workspace !== useApp.getState().settings?.workspace) return;
        useApp.getState().setConfirmationError(e.payload.login, e.payload.message);
      }),
    );
    unlisteners.push(
      listen<{ workspace: string; login: string; count: number; items: Confirmation[] }>(
        AUTH_CONFIRMS_EVENT,
        (e) => {
          if (e.payload.workspace !== useApp.getState().settings?.workspace) return;
          mergeConfirmations(e.payload.login, e.payload.items);
        },
      ),
    );
    unlisteners.push(
      listen<{ workspace: string; login: string; ids: string[] }>(AUTH_AUTO_CONFIRMED_EVENT, (e) => {
        if (e.payload.workspace !== useApp.getState().settings?.workspace) return;
        toast(
          "success",
          t("auth.poller.autoConfirmed", {
            count: e.payload.ids.length,
            login: e.payload.login,
          }),
        );
      }),
    );
    unlisteners.push(
      listen<{ workspace: string; login: string; state: SessionState }>(
        AUTH_SESSION_STATE_EVENT,
        (e) => {
          if (e.payload.workspace !== useApp.getState().settings?.workspace) return;
          setSessionState(e.payload.login, e.payload.state);
        },
      ),
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

  // Align refreshes with expiration, independently of when the view mounted.
  useEffect(() => {
    let cancelled = false;
    const pump = () => {
      if (cancelled) return;
      const state = useApp.getState();
      if (state.authLock?.enabled && !state.authLock.unlocked) return;
      for (const account of state.accounts) {
        if (!account.hasAuthenticator) continue;
        const code = state.codes[account.login];
        if (!code || code.generatedAt + code.periodRemaining <= Date.now() / 1000) void refreshCode(account.login);
      }
    };
    pump();
    const timer = window.setInterval(pump, 1000);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [accounts, authStatus]);

  if (!settings) {
    return (
      <div className="app">
        <AmbientBackground />
        <TitleBar />
        <div className="shell">
          <div className="content">
            <div className="empty boot-state">
              <strong>{bootError ? t("boot.failed") : t("common.booting")}</strong>
              {bootError && <>
                <p>{t("boot.recoveryHint")}</p>
                <details><summary>{t("common.details")}</summary><pre>{bootError}</pre></details>
                <div className="actions">
                  <button onClick={() => bootstrap()}>{t("common.retry")}</button>
                  <button onClick={async () => { try { await api.recoverSettings(); await bootstrap(); } catch (e) { useApp.setState({ bootError: String(e) }); } }}>{t("boot.recover")}</button>
                </div>
              </>}
            </div>
          </div>
        </div>
      </div>
    );
  }

  const showWizard = !settings.firstRunCompleted || !settings.workspace;

  return (
    <div className="app">
      <AmbientBackground />
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
            setView={view => { if (view === "confirmations") showConfirmations(); else if (view === "auth") manageAccess(); else setRoute(view); }}
            toggleLog={() => setLogOpen((v) => !v)}
          />
          <div className="content">
            {route === "settings" ? (
              <SettingsView onClose={() => setRoute("main")} />
            ) : route === "auth" ? (
              <AuthenticatorView initialLogin={authLogin} />
            ) : route === "confirmations" ? (
              <ConfirmationsView initialLogin={confirmationLogin} onManage={manageAccess} />
            ) : (
              <MainView onManage={manageAccess} onConfirmations={showConfirmations} />
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
      {updateInfo && (
        <UpdateModal info={updateInfo} onDismiss={() => setUpdateInfo(null)} />
      )}
      <ToastHost />
    </div>
  );
}
