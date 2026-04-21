import { useEffect, useState } from "react";
import { useApp } from "../state/store";
import { AccountCard } from "../components/AccountCard";
import { AddAccountModal } from "../components/AddAccountModal";
import { ImportAccountsModal } from "../components/ImportAccountsModal";
import { SwitchWarnDialog } from "../components/SwitchWarnDialog";
import { GamePickerModal } from "../components/GamePickerModal";
import { SandboxieInstallModal } from "../components/SandboxieInstallModal";
import { AdminRestartDialog } from "../components/AdminRestartDialog";
import { Spinner } from "../components/Spinner";
import { api, type RunningSandbox } from "../api/tauri";
import { useI18n } from "../i18n";

export function MainView() {
  const {
    accounts,
    healths,
    launch,
    add,
    remove,
    repair,
    refreshAccounts,
    mainSteamError,
    pendingImport,
    clearImportPrompt,
    settings,
    setFavorite,
    refreshAvatar,
    toast,
    launchingLogin,
    setLaunching,
  } = useApp();
  const { t } = useI18n();
  const [showAdd, setShowAdd] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [warnLogin, setWarnLogin] = useState<string | null>(null);
  const [pickGameLogin, setPickGameLogin] = useState<string | null>(null);
  const [adminPrompt, setAdminPrompt] = useState<null | { login: string }>(null);
  const [showSbInstall, setShowSbInstall] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [stoppingLogin, setStoppingLogin] = useState<string | null>(null);
  const [runningSandboxes, setRunningSandboxes] = useState<
    Record<string, RunningSandbox>
  >({});

  useEffect(() => {
    if (pendingImport) {
      setShowImport(true);
      clearImportPrompt();
    }
  }, [pendingImport]);

  // Poll running sandboxes every 2s.
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const list = await api.listRunningSandboxes();
        if (!alive) return;
        const m: Record<string, RunningSandbox> = {};
        for (const r of list) m[r.login] = r;
        setRunningSandboxes(m);
        // Auto-clear launching overlay once the sandbox shows up.
        if (launchingLogin && m[launchingLogin]) {
          setLaunching(null);
        }
      } catch {
        if (!alive) return;
        setRunningSandboxes({});
      }
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [launchingLogin]);

  const tryLaunch = async (login: string) => {
    const mode = settings?.defaultLaunchMode ?? "switch";
    if (mode === "sandbox") {
      try {
        const elevated = await api.isElevated();
        if (!elevated) {
          setAdminPrompt({ login });
          return;
        }
      } catch {}
    }
    if (mode === "switch") {
      try {
        const games = await api.listRunningGames();
        if (games.length > 0) {
          setWarnLogin(login);
          return;
        }
      } catch {}
    }
    await doLaunch(login, mode);
  };

  const doLaunch = async (login: string, mode: "switch" | "sandbox") => {
    setLaunching(login);
    toast("info", t("main.launchTriggered", { login }));
    try {
      await launch(login, mode);
      // Auto-clear after 2.5s if the sandbox poll didn't pick it up
      // (switch mode never appears in runningSandboxes).
      window.setTimeout(() => {
        const cur = useApp.getState().launchingLogin;
        if (cur === login) setLaunching(null);
      }, 2500);
    } catch (e: any) {
      setLaunching(null);
      const msg = String(e);
      if (msg.includes("NEED_ADMIN")) {
        setAdminPrompt({ login });
        return;
      }
      if (msg.toLowerCase().includes("sandboxie") && msg.toLowerCase().includes("not installed")) {
        setShowSbInstall(true);
        toast("error", t("main.launchFailed", { msg }));
        return;
      }
      toast("error", t("main.launchFailed", { msg }));
    }
  };

  const stopSandbox = async (login: string) => {
    setStoppingLogin(login);
    try {
      await api.stopSandbox(login);
      setRunningSandboxes((prev) => {
        const next = { ...prev };
        delete next[login];
        return next;
      });
      toast("success", t("sandbox.stopped", { login }));
    } catch (e: any) {
      toast("error", String(e));
    } finally {
      setStoppingLogin(null);
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await refreshAccounts();
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <div className="main">
      {mainSteamError && (
        <div className="err-banner">
          {t("main.steamError", { msg: mainSteamError })}
        </div>
      )}

      <div className="toolbar">
        <h1 className="section-title">{t("main.accounts")}</h1>
        <div className="spacer" />
        <button className="xs" onClick={handleRefresh} disabled={refreshing}>
          {refreshing ? (
            <span className="busy-chip">
              <Spinner size="xs" inline /> {t("main.refreshing")}
            </span>
          ) : (
            t("common.refresh")
          )}
        </button>
        <button className="xs" onClick={() => setShowImport(true)}>{t("main.import")}</button>
        <button className="xs primary" onClick={() => setShowAdd(true)}>
          {t("main.add")}
        </button>
      </div>

      {accounts.length === 0 ? (
        <div className="empty">
          {t("main.empty")}<small>{t("main.emptyHint")}</small>
        </div>
      ) : (
        <div className="grid">
          {accounts.map((a) => (
            <AccountCard
              key={a.login}
              account={a}
              health={healths[a.login]}
              runningSandbox={runningSandboxes[a.login]}
              launching={launchingLogin === a.login}
              stopping={stoppingLogin === a.login}
              onLaunch={tryLaunch}
              onStopSandbox={stopSandbox}
              onRepair={async (l) => {
                try {
                  await repair(l);
                  toast("success", t("main.repaired", { login: l }));
                } catch (e: any) {
                  toast("error", String(e));
                }
              }}
              onRemove={async (l) => {
                // Confirm only here — AccountCard no longer double-prompts.
                if (confirm(t("main.confirmRemove", { login: l }))) {
                  try {
                    await remove(l, true);
                    toast("success", t("main.removed", { login: l }));
                  } catch (e: any) {
                    toast("error", String(e));
                  }
                }
              }}
              onToggleFavorite={async (l, v) => {
                try { await setFavorite(l, v); } catch (e: any) { toast("error", String(e)); }
              }}
              onPickGame={(l) => setPickGameLogin(l)}
              onRefreshAvatar={(l) => refreshAvatar(l)}
            />
          ))}
        </div>
      )}

      <AddAccountModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        onSubmit={async (login, display) => {
          await add(login, display);
        }}
      />
      <ImportAccountsModal
        open={showImport}
        onClose={() => setShowImport(false)}
        existingLogins={new Set(accounts.map((a) => a.login))}
        onImported={() => refreshAccounts()}
      />
      <SwitchWarnDialog
        open={!!warnLogin}
        login={warnLogin}
        onClose={() => setWarnLogin(null)}
        onConfirmed={async () => {
          if (warnLogin) await doLaunch(warnLogin, "switch");
        }}
      />
      <GamePickerModal
        open={!!pickGameLogin}
        login={pickGameLogin}
        defaultMode={settings?.defaultLaunchMode ?? "switch"}
        onClose={() => setPickGameLogin(null)}
      />
      <AdminRestartDialog
        open={!!adminPrompt}
        onCancel={() => setAdminPrompt(null)}
        onContinue={async () => {
          setAdminPrompt(null);
          try { await api.relaunchAsAdmin(); } catch (e: any) { toast("error", String(e)); }
        }}
      />
      <SandboxieInstallModal
        open={showSbInstall}
        onClose={() => setShowSbInstall(false)}
      />
    </div>
  );
}
