import { useState } from "react";
import { useI18n } from "../i18n";

interface Props {
  open: boolean;
  onClose(): void;
  onSubmit(login: string, display: string | null): Promise<void>;
}

export function AddAccountModal({ open, onClose, onSubmit }: Props) {
  const { t } = useI18n();
  const [login, setLogin] = useState("");
  const [display, setDisplay] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  if (!open) return null;

  const submit = async () => {
    if (!/^[a-zA-Z0-9_.\-]{1,64}$/.test(login)) {
      setErr(t("add.invalidLogin"));
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await onSubmit(login.trim(), display.trim() || null);
      setLogin("");
      setDisplay("");
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
        <h2>{t("add.title")}</h2>
        <div className="row">
          <label>{t("add.login")}</label>
          <input
            type="text"
            value={login}
            placeholder={t("add.loginPlaceholder")}
            onChange={(e) => setLogin(e.target.value)}
            autoFocus
          />
        </div>
        <div className="row">
          <label>{t("add.display")}</label>
          <input
            type="text"
            value={display}
            placeholder={t("add.displayPlaceholder")}
            onChange={(e) => setDisplay(e.target.value)}
          />
        </div>
        {err && <div className="err-banner">{err}</div>}
        <div className="buttons">
          <button onClick={onClose} disabled={busy}>{t("common.cancel")}</button>
          <button className="primary" onClick={submit} disabled={busy || !login}>
            {busy ? t("add.creating") : t("add.create")}
          </button>
        </div>
      </div>
    </div>
  );
}
