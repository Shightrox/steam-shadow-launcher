import { useGuardCode, copyGuardCode } from "../state/guardCode";
import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, pickFile, type Account, type AccountHealth, type RunningSandbox } from "../api/tauri";
import { HealthBadge } from "./HealthBadge";
import { useI18n } from "../i18n";
import { useApp } from "../state/store";
import { Spinner } from "./Spinner";
import { ContextMenu, type ContextMenuEntry } from "./ContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";
import { Icon } from "./Icon";

interface Props {
  account: Account;
  selected: boolean;
  onSelect(): void;
  onManage(): void;
  onConfirmations(): void;
  health?: AccountHealth;
  runningSandbox?: RunningSandbox;
  launching?: boolean;
  stopping?: boolean;
  onLaunch(login: string): void;
  onRepair(login: string): void;
  onRemove(login: string): void;
  onToggleFavorite?(login: string, value: boolean): void;
  onStopSandbox?(login: string): void;
  onPickGame?(login: string): void;
  onRefreshAvatar?(login: string): void;
}

function fmtUptime(secsAgo: number): string {
  if (secsAgo <= 0) return "—";
  const m = Math.floor(secsAgo / 60);
  if (m < 1) return `${secsAgo}s`;
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return `${h}h${rm}m`;
}

export function AccountCard({
  account,
  selected,
  onSelect,
  onManage,
  onConfirmations,
  health,
  runningSandbox,
  launching,
  stopping,
  onLaunch,
  onRepair,
  onRemove,
  onToggleFavorite,
  onStopSandbox,
  onPickGame,
  onRefreshAvatar,
}: Props) {
  const { t, lang } = useI18n();
  const { toast, importMafile, removeAuthenticator } = useApp();
  const [busy, setBusy] = useState(false);
  const [avatarBusy, setAvatarBusy] = useState(false);
  const [failedAvatar, setFailedAvatar] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [remove2faOpen, setRemove2faOpen] = useState(false);
  const [remove2faBusy, setRemove2faBusy] = useState(false);
  const auth = useApp(s => s.authStatus[account.login]);
  const session = useApp(s => s.sessionStates[account.login]);
  const locked = useApp(s => !!s.authLock?.enabled && !s.authLock.unlocked);
  const confirmations = useApp(s => s.confirmations[account.login]);
  const confirmationError = useApp(s => s.confErrors[account.login]);
  const has2fa = account.hasAuthenticator || auth?.hasAuthenticator;
  const { code, remaining } = useGuardCode(has2fa ? account.login : null);
  const needsLogin = session === "needs_relogin" || session === "no_session";
  const autoReady = auth?.hasSavedPassword && auth.autoLogin?.state === "ready";
  const accessLabel = locked ? t("design.lockedShort") : !has2fa ? t("design.addGuard")
    : needsLogin && !autoReady ? t("design.signInShort") : autoReady ? t("design.autoReady")
    : session === "ok" ? t("design.sessionOk") : t("design.access");

  const launch = async () => {
    onSelect();
    setBusy(true);
    try {
      await onLaunch(account.login);
    } finally {
      setBusy(false);
    }
  };

  const lastLaunchDate = new Date(Number(account.lastLaunchAt) * 1000);
  const lastLaunch = account.lastLaunchAt && Number.isFinite(lastLaunchDate.getTime())
    ? lastLaunchDate.toLocaleString(lang === "ru" ? "ru-RU" : "en-GB", { day: "2-digit", month: "2-digit", year: "2-digit", hour: "2-digit", minute: "2-digit" })
    : t("common.never");

  const avatarSrc = account.avatarPath
    ? `${convertFileSrc(account.avatarPath)}?v=${account.lastLaunchAt ?? "0"}`
    : null;

  const initial = (account.displayName || account.login || "?")
    .trim()
    .charAt(0)
    .toUpperCase();

  const inSandbox = !!runningSandbox;
  const uptimeSec = inSandbox && runningSandbox?.startedAt
    ? Math.max(0, Math.floor(Date.now() / 1000) - runningSandbox.startedAt)
    : 0;

  const doRefreshAvatar = async () => {
    if (!onRefreshAvatar) return;
    setAvatarBusy(true);
    try {
      await onRefreshAvatar(account.login);
    } finally {
      setAvatarBusy(false);
    }
  };

  const buildMenu = (): ContextMenuEntry[] => {
    const sid = account.steamId;
    const items: ContextMenuEntry[] = [
      { label: t("auth.confirmations"), disabled: !has2fa || locked, onClick: onConfirmations },
      { label: t("design.manageAccess"), onClick: onManage },
      { divider: true },
      {
        label: t("card.openProfile"),
        disabled: !sid,
        onClick: () => {
          if (sid) api.openUrl(`https://steamcommunity.com/profiles/${sid}`).catch((e) => toast("error", String(e)));
        },
      },
      {
        label: t("card.openInventory"),
        disabled: !sid,
        onClick: () => {
          if (sid) api.openUrl(`https://steamcommunity.com/profiles/${sid}/inventory`).catch((e) => toast("error", String(e)));
        },
      },
      { divider: true },
      {
        label: account.favorite ? t("card.fav.remove") : t("card.fav.add"),
        disabled: !onToggleFavorite,
        onClick: () => onToggleFavorite?.(account.login, !account.favorite),
      },
      {
        label: t("card.refreshAvatar"),
        disabled: !onRefreshAvatar,
        onClick: () => void doRefreshAvatar(),
      },
      {
        label: t("card.createShortcut"),
        onClick: async () => {
          try {
            const p = await api.createAccountShortcut(account.login);
            toast("success", t("card.shortcut.created", { path: p }));
          } catch (e: any) {
            toast("error", String(e));
          }
        },
      },
      { divider: true },
      {
        label: t("main.repair"),
        onClick: () => onRepair(account.login),
      },
      {
        label: t("main.remove"),
        danger: true,
        onClick: () => onRemove(account.login),
      },
    ];
    // P11: authenticator actions
    items.splice(items.length - 2, 0, { divider: true });
    if (has2fa) {
      items.splice(items.length - 2, 0, {
        label: t("card.twofaCopy"),
        disabled: !code,
        onClick: async () => {
          if (!code) return;
          try {
            await copyGuardCode(account.login);
            toast("success", t("auth.copied"));
          } catch (e: any) {
            toast("error", String(e));
          }
        },
      });
      items.splice(items.length - 2, 0, {
        label: t("card.twofaRemove"),
        danger: true,
        onClick: () => setRemove2faOpen(true),
      });
    } else {
      items.splice(items.length - 2, 0, {
        label: t("card.twofaImport"),
        onClick: async () => {
          try {
            const p = await pickFile(t("auth.import"), ["maFile", "json"]);
            if (!p) return;
            await importMafile(account.login, p);
            toast("success", t("auth.import.success", { login: account.login }));
          } catch (e: any) {
            toast("error", String(e));
          }
        },
      });
    }
    return items;
  };

  return (
    <div
      className={`card account-card${selected ? " selected" : ""}${launching ? " launching" : ""}`}
      data-login={account.login}
      onClick={e => { if (!(e.target as HTMLElement).closest("button, input, [role=menu]")) onSelect(); }}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      <div className="head">
        <div className={`avatar${avatarBusy ? " busy" : ""}`} aria-hidden="true">
          {avatarSrc && failedAvatar !== avatarSrc ? (
            <img
              src={avatarSrc}
              alt=""
              draggable={false}
              onError={() => setFailedAvatar(avatarSrc)}
            />
          ) : (
            <span className="avatar-fallback">{initial}</span>
          )}
        </div>
        <div style={{ minWidth: 0, flex: 1 }}>
          <button
            className="title"
            aria-pressed={selected}
            title={account.displayName || account.login}
            onClick={onSelect}
            style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {account.displayName || account.login}
          </button>
          {account.displayName && <div className="sub">@{account.login}</div>}
        </div>
        {onToggleFavorite && (
          <button
            className="fav-btn"
            onClick={() => onToggleFavorite(account.login, !account.favorite)}
            title={account.favorite ? t("card.fav.remove") : t("card.fav.add")}
            aria-label={account.favorite ? t("card.fav.remove") : t("card.fav.add")}
            aria-pressed={account.favorite}
          >
            {account.favorite ? "★" : "☆"}
          </button>
        )}
        <button className="xs icon-button account-more" aria-label={`${t("design.more")} ${account.displayName || account.login}`} aria-haspopup="menu" aria-expanded={!!menu} onClick={e => { const r = e.currentTarget.getBoundingClientRect(); setMenu({ x: r.left, y: r.bottom + 4 }); }}><Icon name="more" size={14} /></button>
      </div>
      <div className="card-guard-row">
        <div className="card-guard">
          {has2fa && !locked ? <>
            <button className={`card-guard-code${remaining <= 5 ? " expiring" : ""}`} disabled={!code} aria-label={`${t("auth.copy")} ${account.login}`} onClick={async () => {
              try { await copyGuardCode(account.login); toast("success", t("auth.copied")); }
              catch (e) { toast("error", String(e)); }
            }}><span>{code?.code ?? "·····"}</span><Icon name="copy" size={12} /></button>
            <span className="card-guard-timer">{remaining}{t("design.seconds")}</span>
            <div className={`guard-progress${remaining <= 5 ? " expiring" : ""}`} aria-hidden="true"><span style={{ width: `${remaining / 30 * 100}%` }} /></div>
          </> : <button className="guard-setup ghost xs" onClick={onManage}><Icon name="shield" size={12} />{locked ? t("auth.security.unlock") : t("design.addGuard")}</button>}
        </div>
        <div className="account-state">{inSandbox ? <span className="sandbox-running" title={t("card.inSandbox")}><span className="state-dot" />Sandbox {fmtUptime(uptimeSec)}</span> : <HealthBadge health={health} compact />}</div>
      </div>
      <div className="card-meta"><span>{t("main.lastLaunch")}<b>{lastLaunch}</b></span><span title={t("card.launchCount")}>{t("design.launches", { count: account.launchCount })}</span></div>
      <div className="actions">
        {inSandbox && onStopSandbox ? (
          <button
            className="stop-sandbox"
            disabled={stopping}
            onClick={() => onStopSandbox(account.login)}
          >
            {stopping ? (
              <span className="busy-chip">
                <Spinner size="xs" inline />
              </span>
            ) : (
              t("card.stopSandbox")
            )}
          </button>
        ) : (
          <button
            className="primary"
            disabled={busy || launching}
            onClick={launch}
            title={!health?.ready ? t("main.notReadyHint") : ""}
          >
            {t("main.launch")}
          </button>
        )}
        {!inSandbox && onPickGame && (
          <button
            className="xs"
            disabled={launching}
            onClick={() => onPickGame(account.login)}
            title={t("card.launchGame")}
          >
            {t("card.launchGame")}
          </button>
        )}
      </div>
      <div className="card-bottom">
        {has2fa ? <button className="card-confirmations" onClick={locked ? onManage : onConfirmations} title={locked ? t("auth.lockedHint") : confirmationError || t("design.checkConfirmations")}>
          {t("auth.confirmations")} <b>{locked ? "—" : confirmationError ? "!" : confirmations?.length ?? "—"}</b><Icon name="arrow" size={11} />
        </button> : <span className="dim">{t("auth.noAuthShort")}</span>}
        <button className={`card-access${has2fa && needsLogin && !autoReady ? " warning" : ""}`} onClick={onManage} title={locked ? t("auth.lockedHint") : auth?.hasSavedPassword ? t(`auth.auto.${auth.autoLogin?.state ?? "unavailable"}`) : t("design.manageAccess")}>{accessLabel}</button>
      </div>
      {launching && (
        <div className="launch-fog" aria-hidden="true">
          <span className="launch-text">
            {t("card.launching")}
            <span className="blink-cursor">_</span>
          </span>
        </div>
      )}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={buildMenu()}
          onClose={() => setMenu(null)}
        />
      )}
      <ConfirmDialog
        open={remove2faOpen}
        title={t("auth.removeConfirmTitle", { login: account.login })}
        body={t("auth.removeConfirmBody")}
        bullets={[
          t("auth.removeConfirmBullet1"),
          t("auth.removeConfirmBullet2"),
          t("auth.removeConfirmBullet3"),
        ]}
        requireText={account.login}
        requireHint={t("confirm.typeLoginToConfirm", { login: account.login })}
        confirmLabel={t("auth.removeConfirmButton")}
        busy={remove2faBusy}
        onCancel={() => !remove2faBusy && setRemove2faOpen(false)}
        onConfirm={async () => {
          setRemove2faBusy(true);
          try {
            await removeAuthenticator(account.login);
            toast("success", t("auth.remove"));
            setRemove2faOpen(false);
          } catch (e: any) {
            toast("error", String(e));
          } finally {
            setRemove2faBusy(false);
          }
        }}
      />
    </div>
  );
}
