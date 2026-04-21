import { useEffect, useState } from "react";
import { api, type RunningGame } from "../api/tauri";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  login: string | null;
  onClose(): void;
  onConfirmed(): Promise<void>;
}

export function SwitchWarnDialog({ open, login, onClose, onConfirmed }: Props) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [game, setGame] = useState<RunningGame | null>(null);

  useEffect(() => {
    if (!open) return;
    api.listRunningGames().then((g) => setGame(g[0] || null));
  }, [open]);

  if (!open || !login) return null;

  const proceed = async () => {
    setBusy(true);
    try {
      await onConfirmed();
      onClose();
    } catch (e: any) {
      useApp.getState().log("error", String(e));
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("switchWarn.title")}</h2>
        <p style={{ color: "var(--fg)", fontSize: 11 }}>
          {t("switchWarn.body", { game: game?.exeName || "?" })}
        </p>
        <div className="buttons">
          <button onClick={onClose} disabled={busy}>{t("common.cancel")}</button>
          <button className="primary danger" onClick={proceed} disabled={busy}>
            {busy ? "…" : t("switchWarn.go")}
          </button>
        </div>
      </div>
    </div>
  );
}
