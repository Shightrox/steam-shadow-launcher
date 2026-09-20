import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../state/store";
import { AccountCard } from "../components/AccountCard";
import { AddAccountModal } from "../components/AddAccountModal";
import { ImportAccountsModal } from "../components/ImportAccountsModal";
import { SwitchWarnDialog } from "../components/SwitchWarnDialog";
import { GamePickerModal } from "../components/GamePickerModal";
import { SandboxieInstallModal } from "../components/SandboxieInstallModal";
import { AdminRestartDialog } from "../components/AdminRestartDialog";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Spinner } from "../components/Spinner";
import { api, type RunningSandbox } from "../api/tauri";
import { useI18n } from "../i18n";
import { LaunchModeSwitch } from "../components/LaunchModeSwitch";
import { Icon } from "../components/Icon";

export function MainView({ onManage, onConfirmations }: { onManage(login: string): void; onConfirmations(login?: string): void }) {
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
  const existingLogins = useMemo(() => new Set(accounts.map((a) => a.login)), [accounts]);
  const launchGuard = useRef(false);
  const [warnAppId, setWarnAppId] = useState<number | undefined>();
  const [showAdd, setShowAdd] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [warnLogin, setWarnLogin] = useState<string | null>(null);
  const [pickGameLogin, setPickGameLogin] = useState<string | null>(null);
  const [adminPrompt, setAdminPrompt] = useState<null | { login: string }>(null);
  const [showSbInstall, setShowSbInstall] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [stoppingLogin, setStoppingLogin] = useState<string | null>(null);
  const [removeFor, setRemoveFor] = useState<string | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [compact, setCompact] = useState(() => localStorage.getItem("shadow.accountLayout") === "rows");
  const active = accounts.find(a => a.login === selected) ?? accounts[0];
  const filtered = accounts.filter(a => `${a.displayName ?? ""} ${a.login}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
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

  const tryLaunch = async (login: string, appid?: number) => {
    if (launchGuard.current || useApp.getState().launchingLogin) return;
    launchGuard.current = true;
    try {
    const mode = settings?.defaultLaunchMode ?? "switch";
    if (mode === "sandbox") {
      try {
        const elevated = await api.isElevated();
        if (!elevated) {
          setAdminPrompt({ login });
          return;
        }
      } catch (e) { toast("error", String(e)); return; }
    }
    if (mode === "switch") {
      try {
        const games = await api.listRunningGames();
        if (games.length > 0) {
          setWarnAppId(appid);
          setWarnLogin(login);
          return;
        }
      } catch (e) { toast("error", String(e)); return; }
    }
    await doLaunch(login, mode, appid);
    } finally { launchGuard.current = false; }
  };

  const doLaunch = async (login: string, mode: "switch" | "sandbox", appid?: number) => {
    setLaunching(login);
    toast("info", t("main.launchTriggered", { login }));
    try {
      if (appid !== undefined) await api.launchGame(login, appid, mode);
      else await launch(login, mode);
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
        <div className="page-heading"><h1>{t("main.accounts")}<span className="heading-count">{accounts.length}</span></h1></div>
        <div className="spacer" />
        <button className="xs ghost icon-button" aria-label={t("common.refresh")} title={t("common.refresh")} onClick={handleRefresh} disabled={refreshing}>
          {refreshing ? (
            <span className="busy-chip">
              <Spinner size="xs" inline />
            </span>
          ) : (
            <Icon name="refresh" size={14} />
          )}
        </button>
        <button className="xs ghost" onClick={() => setShowImport(true)}>{t("main.import")}</button>
        <button className="xs" onClick={() => setShowAdd(true)}>
          {t("main.add")}
        </button>
      </div>

      <div className="account-tools"><LaunchModeSwitch /><div className="spacer" />
        <label className="account-search"><Icon name="search" size={16} /><input type="search" value={query} onChange={e => setQuery(e.target.value)} aria-label={t("design.search")} placeholder={t("design.search")} /></label>
        <div className="layout-switch" role="group" aria-label={t("design.layout")}>
          <button className="icon-button" aria-label={t("design.cards")} aria-pressed={!compact} onClick={() => { setCompact(false); localStorage.setItem("shadow.accountLayout", "cards"); }}><Icon name="grid" size={16} /></button>
          <button className="icon-button" aria-label={t("design.rows")} aria-pressed={compact} onClick={() => { setCompact(true); localStorage.setItem("shadow.accountLayout", "rows"); }}><Icon name="rows" size={16} /></button>
        </div>
      </div>

      {accounts.length === 0 ? (
        <div className="empty">
          {t("main.empty")}<small>{t("main.emptyHint")}</small>
        </div>
      ) : (
        <div className={`grid account-grid${compact ? " account-rows" : ""}`}>
          {filtered.length === 0 && <div className="empty">{t("design.noMatches")}</div>}
          {filtered.map((a) => (
            <AccountCard
              key={a.login}
              account={a}
              selected={active?.login === a.login}
              onSelect={() => setSelected(a.login)}
              onManage={() => onManage(a.login)}
              onConfirmations={() => onConfirmations(a.login)}
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
                // Open dialog with type-to-confirm; actual deletion runs in
                // the dialog's onConfirm callback below.
                setRemoveFor(l);
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
        existingLogins={existingLogins}
        onImported={() => refreshAccounts()}
      />
      <SwitchWarnDialog
        open={!!warnLogin}
        login={warnLogin}
        onClose={() => setWarnLogin(null)}
        onConfirmed={async () => {
          if (warnLogin) await doLaunch(warnLogin, "switch", warnAppId);
        }}
      />
      <GamePickerModal
        open={!!pickGameLogin}
        login={pickGameLogin}
        onLaunch={tryLaunch}
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
      <ConfirmDialog
        open={!!removeFor}
        title={t("main.removeConfirmTitle", { login: removeFor ?? "" })}
        body={t("main.removeConfirmBody")}
        bullets={[
          t("main.removeConfirmBullet1"),
          t("main.removeConfirmBullet2"),
          t("main.removeConfirmBullet3"),
        ]}
        requireText={removeFor ?? ""}
        requireHint={t("confirm.typeLoginToConfirm", { login: removeFor ?? "" })}
        confirmLabel={t("main.removeConfirmButton")}
        busy={removeBusy}
        onCancel={() => !removeBusy && setRemoveFor(null)}
        onConfirm={async () => {
          if (!removeFor) return;
          setRemoveBusy(true);
          try {
            await remove(removeFor, true);
            toast("success", t("main.removed", { login: removeFor }));
            setRemoveFor(null);
          } catch (e: any) {
            toast("error", String(e));
          } finally {
            setRemoveBusy(false);
          }
        }}
      />
    </div>
  );
}
