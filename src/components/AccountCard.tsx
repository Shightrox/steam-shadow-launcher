import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, pickFile, type Account, type AccountHealth, type RunningSandbox } from "../api/tauri";
import { HealthBadge } from "./HealthBadge";
import { useI18n } from "../i18n";
import { useApp } from "../state/store";
import { Spinner } from "./Spinner";
import { ContextMenu, type ContextMenuEntry } from "./ContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";

interface Props {
  account: Account;
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
  const { t } = useI18n();
  const { toast, codes, importMafile, removeAuthenticator } = useApp();
  const [busy, setBusy] = useState(false);
  const [avatarBusy, setAvatarBusy] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [remove2faOpen, setRemove2faOpen] = useState(false);
  const [remove2faBusy, setRemove2faBusy] = useState(false);
  const code = codes[account.login];
  const has2fa = account.hasAuthenticator;

  const launch = async () => {
    setBusy(true);
    try {
      await onLaunch(account.login);
    } finally {
      setBusy(false);
    }
  };

  const lastLaunch = account.lastLaunchAt
    ? new Date(parseInt(account.lastLaunchAt) * 1000).toLocaleString()
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
            await navigator.clipboard.writeText(code.code);
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
      className={`card${launching ? " launching" : ""}`}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      <div className="head">
        <div className={`avatar${avatarBusy ? " busy" : ""}`} aria-hidden="true">
          {avatarSrc ? (
            <img
              src={avatarSrc}
              alt=""
              draggable={false}
              onError={(e) => {
                (e.currentTarget as HTMLImageElement).style.display = "none";
              }}
            />
          ) : (
            <span className="avatar-fallback">{initial}</span>
          )}
        </div>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div
            className="title"
            style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {account.displayName || account.login}
          </div>
          {account.displayName && <div className="sub">@{account.login}</div>}
        </div>
        {onToggleFavorite && (
          <button
            className="fav-btn"
            onClick={() => onToggleFavorite(account.login, !account.favorite)}
            title={account.favorite ? t("card.fav.remove") : t("card.fav.add")}
            aria-label="favorite"
          >
            {account.favorite ? "★" : "☆"}
          </button>
        )}
        <HealthBadge health={health} />
      </div>
      <div className="sub">
        {t("main.lastLaunch")} {lastLaunch}
        {account.launchCount > 0 && (
          <span style={{ marginLeft: 8 }}>
            · {t("card.launchCount")} {account.launchCount}
          </span>
        )}
      </div>
      {inSandbox && (
        <div className="sub" style={{ color: "#66ffcc" }}>
          ▶ {t("card.inSandbox")} · {fmtUptime(uptimeSec)}
        </div>
      )}
      {has2fa && code && (
        <div className="twofa-widget" title={t("auth.code")}>
          <span className="twofa-label">{t("card.twofa")}</span>
          <span
            className={`twofa-code${code.periodRemaining <= 5 ? " pulse" : ""}`}
          >
            {code.code}
          </span>
          <span className="twofa-countdown">{code.periodRemaining}s</span>
          <button
            className="xs"
            onClick={async () => {
              try {
                await navigator.clipboard.writeText(code.code);
                toast("success", t("auth.copied"));
              } catch (e: any) {
                toast("error", String(e));
              }
            }}
            title={t("auth.copy")}
          >
            ⎘
          </button>
        </div>
      )}
      <div className="actions">
        {inSandbox && onStopSandbox ? (
          <button
            className="primary danger"
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
        <button className="xs" onClick={() => onRepair(account.login)}>
          {t("main.repair")}
        </button>
        <button className="xs danger ghost" onClick={() => onRemove(account.login)}>
          {t("main.remove")}
        </button>
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
