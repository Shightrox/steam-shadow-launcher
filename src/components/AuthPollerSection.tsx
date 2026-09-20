import { useEffect, useState } from "react";
import { api, type Settings } from "../api/tauri";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";

/// Settings panel section: background confirmation poller + auto-confirm flags.
export function AuthPollerSection() {
  const { t } = useI18n();
  const { settings, log, toast } = useApp();
  const [busy, setBusy] = useState(false);
  const [local, setLocal] = useState<Settings | null>(settings);

  useEffect(() => {
    setLocal(settings);
  }, [settings]);

  if (!local) return null;

  const apply = async (patch: Partial<Settings>) => {
    const next: Settings = { ...local, ...patch };
    setLocal(next);
    setBusy(true);
    try {
      await api.authPollerConfigure({
        enabled: next.authPollerEnabled,
        interval: next.authPollerInterval,
        autoConfirmTrades: next.authAutoConfirmTrades,
        autoConfirmMarket: next.authAutoConfirmMarket,
      });
      useApp.setState(s => ({ settings: s.settings ? { ...s.settings, authPollerEnabled: next.authPollerEnabled, authPollerInterval: next.authPollerInterval, authAutoConfirmTrades: next.authAutoConfirmTrades, authAutoConfirmMarket: next.authAutoConfirmMarket } : null }));
    } catch (e: any) {
      setLocal(useApp.getState().settings);
      toast("error", String(e));
      log("error", `poller configure: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const pokeNow = async () => {
    setBusy(true);
    try {
      await api.authPollerPoke();
      toast("info", t("auth.poller.checked"));
    } catch (e: any) {
      toast("error", String(e));
      log("warn", `poller poke: ${e}`);
    } finally { setBusy(false); }
  };

  const showWarning = local.authAutoConfirmTrades || local.authAutoConfirmMarket;

  return (
    <div className="card compact">
      <div className="title">{t("auth.poller.title")}</div>
      <div className="sub">{t("auth.poller.subtitle")}</div>

      <label className="auth-sec-row">
        <input
          type="checkbox"
          checked={local.authPollerEnabled}
          disabled={busy}
          onChange={(e) => apply({ authPollerEnabled: e.target.checked })}
        />
        <span>{t("auth.poller.enable")}</span>
      </label>

      <div className="auth-sec-row">
        <label>{t("auth.poller.interval")}</label>
        <input
          type="number"
          min={15}
          max={600}
          value={local.authPollerInterval}
          disabled={busy || !local.authPollerEnabled}
          onChange={(e) =>
            setLocal({ ...local, authPollerInterval: Number(e.target.value) })
          }
          onBlur={() =>
            apply({
              authPollerInterval: Math.min(
                600,
                Math.max(15, local.authPollerInterval || 60),
              ),
            })
          }
          style={{ width: 80 }}
        />
        <button className="xs ghost" onClick={pokeNow} disabled={busy}>
          {t(busy ? "common.booting" : "auth.poller.pokeNow")}
        </button>
      </div>

      <label className="auth-sec-row">
        <input
          type="checkbox"
          checked={local.authAutoConfirmTrades}
          disabled={busy}
          onChange={(e) => apply({ authAutoConfirmTrades: e.target.checked })}
        />
        <span>{t("auth.poller.autoTrades")}</span>
      </label>

      <label className="auth-sec-row">
        <input
          type="checkbox"
          checked={local.authAutoConfirmMarket}
          disabled={busy}
          onChange={(e) => apply({ authAutoConfirmMarket: e.target.checked })}
        />
        <span>{t("auth.poller.autoMarket")}</span>
      </label>

      {showWarning && <div className="auth-err">{t("auth.poller.warning")}</div>}
    </div>
  );
}
