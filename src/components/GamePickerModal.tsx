import { useEffect, useMemo, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type InstalledGame, type LaunchMode } from "../api/tauri";
import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  login: string | null;
  defaultMode: LaunchMode;
  onClose(): void;
  onLaunched?(): void;
}

export function GamePickerModal({ open, login, defaultMode, onClose, onLaunched }: Props) {
  const { t } = useI18n();
  const [games, setGames] = useState<InstalledGame[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [busyId, setBusyId] = useState<number | null>(null);

  useEffect(() => {
    if (!open || !login) return;
    setLoading(true);
    setErr(null);
    setQ("");
    api
      .listAccountGames(login)
      .then((g) => setGames(g))
      .catch((e) => setErr(String(e)))
      .finally(() => setLoading(false));
  }, [open, login]);

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return games;
    return games.filter(
      (g) => g.name.toLowerCase().includes(s) || String(g.appid).includes(s)
    );
  }, [games, q]);

  if (!open) return null;

  const launch = async (g: InstalledGame) => {
    if (!login) return;
    setBusyId(g.appid);
    try {
      await api.launchGame(login, g.appid, defaultMode);
      onLaunched?.();
      onClose();
    } catch (e: any) {
      alert(String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 720, maxWidth: "94vw" }}
      >
        <h2>{t("games.title")}</h2>
        <div className="row" style={{ marginBottom: 8 }}>
          <input
            autoFocus
            placeholder={t("games.search")}
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
        </div>
        {loading && <div className="sub">{t("games.loading")}</div>}
        {err && <div className="sub" style={{ color: "var(--danger)" }}>{err}</div>}
        {!loading && !err && filtered.length === 0 && (
          <div className="empty"><span>{t("games.empty")}</span></div>
        )}
        <div className="game-grid">
          {filtered.map((g) => (
            <button
              key={g.appid}
              className="game-tile"
              onClick={() => launch(g)}
              disabled={busyId === g.appid}
              title={`${g.name} · ${g.appid}`}
            >
              <div className="game-cover">
                {g.iconPath ? (
                  <img src={convertFileSrc(g.iconPath)} alt="" draggable={false} />
                ) : (
                  <span className="game-cover-fallback">
                    {g.name.charAt(0).toUpperCase()}
                  </span>
                )}
              </div>
              <div className="game-name">{g.name}</div>
            </button>
          ))}
        </div>
        <div className="buttons">
          <button onClick={onClose}>{t("common.close")}</button>
        </div>
      </div>
    </div>
  );
}
