import { useEffect, useState } from "react";
import { api, type DiscoveredAccount } from "../api/tauri";
import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  onClose(): void;
  onImported(): void;
  existingLogins: Set<string>;
}

export function ImportAccountsModal({ open, onClose, onImported, existingLogins }: Props) {
  const { t } = useI18n();
  const [items, setItems] = useState<DiscoveredAccount[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setErr(null);
    setSelected(new Set());
    api.discoverSteamAccounts()
      .then((r) => {
        setItems(r);
        // Pre-select non-imported ones
        setSelected(new Set(r.filter((a) => !existingLogins.has(a.accountName)).map((a) => a.accountName)));
      })
      .catch((e) => setErr(String(e)));
  }, [open, existingLogins]);

  if (!open) return null;

  const allExist = items.length > 0 && items.every((a) => existingLogins.has(a.accountName));
  const newCount = items.filter((a) => !existingLogins.has(a.accountName)).length;

  const toggle = (login: string) => {
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(login)) n.delete(login);
      else n.add(login);
      return n;
    });
  };

  const submit = async () => {
    setBusy(true);
    setErr(null);
    try {
      const personas: Record<string, string> = {};
      for (const it of items) {
        if (selected.has(it.accountName) && it.personaName) {
          personas[it.accountName] = it.personaName;
        }
      }
      await api.importDiscoveredAccounts([...selected], personas);
      onImported();
      onClose();
    } catch (e: any) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("import.title")}</h2>
        {items.length === 0 ? (
          <div className="empty">{t("import.empty")}</div>
        ) : (
          <>
            <div className="hint" style={{ color: "var(--fg-mute)", fontSize: 10, marginBottom: 4 }}>
              {allExist ? t("import.allExist") : t("import.hint")}
            </div>
            <div className="import-list">
              {items.map((a) => {
                const exists = existingLogins.has(a.accountName);
                return (
                  <label key={a.accountName} title={exists ? "already imported" : ""}>
                    <input
                      type="checkbox"
                      checked={selected.has(a.accountName)}
                      onChange={() => toggle(a.accountName)}
                      disabled={exists}
                    />
                    <span className="persona">{a.personaName || a.accountName}</span>
                    <span style={{ color: "var(--fg-mute)" }}>@{a.accountName}</span>
                    {a.mostRecent && <span className="recent">{t("import.recent")}</span>}
                    {exists && <span className="recent" style={{ color: "var(--fg-mute)" }}>{t("import.exists")}</span>}
                  </label>
                );
              })}
            </div>
          </>
        )}
        {err && <div className="err-banner">{err}</div>}
        <div className="buttons">
          <button onClick={onClose} disabled={busy}>{t("common.cancel")}</button>
          <button
            className="primary"
            onClick={submit}
            disabled={busy || selected.size === 0}
          >
            {busy ? "…" : t("import.do")}
          </button>
        </div>
      </div>
    </div>
  );
}
