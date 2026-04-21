import { useState } from "react";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { Spinner } from "./Spinner";

/// Settings panel section: master password (Argon2id + AES-GCM) for all
/// stored maFiles.
export function AuthSecuritySection() {
  const { t } = useI18n();
  const { authLock, setMasterPassword, unlockAuth, lockAuth } = useApp();

  const [mode, setMode] = useState<"idle" | "enable" | "change" | "disable">("idle");
  const [oldPw, setOldPw] = useState("");
  const [newPw, setNewPw] = useState("");
  const [newPw2, setNewPw2] = useState("");
  const [unlockPw, setUnlockPw] = useState("");
  const [ackWarn, setAckWarn] = useState(false);
  const [busy, setBusy] = useState(false);

  const enabled = !!authLock?.enabled;
  const unlocked = !!authLock?.unlocked;
  const hasFiles = !!authLock?.hasEncryptedFiles;

  const reset = () => {
    setMode("idle");
    setOldPw("");
    setNewPw("");
    setNewPw2("");
    setAckWarn(false);
  };

  const doApply = async () => {
    setBusy(true);
    try {
      if (mode === "disable") {
        const ok = await setMasterPassword(oldPw || null, null);
        if (ok) reset();
      } else {
        if (newPw.length < 6) {
          alert(t("auth.security.short"));
          return;
        }
        if (newPw !== newPw2) {
          alert(t("auth.security.mismatch"));
          return;
        }
        const ok = await setMasterPassword(
          mode === "change" ? oldPw || null : null,
          newPw,
        );
        if (ok) reset();
      }
    } finally {
      setBusy(false);
    }
  };

  const doUnlock = async () => {
    if (!unlockPw) return;
    setBusy(true);
    try {
      const ok = await unlockAuth(unlockPw);
      if (ok) setUnlockPw("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card compact">
      <div className="title">{t("auth.security.title")}</div>
      <div className="sub">{t("auth.security.subtitle")}</div>
      <div className="sub">
        {t("auth.security.state")}{" "}
        {enabled
          ? unlocked
            ? t("auth.security.stateUnlocked")
            : t("auth.security.stateLocked")
          : t("auth.security.stateOff")}
      </div>

      {enabled && !unlocked && hasFiles && (
        <div className="auth-sec-row">
          <input
            type="password"
            value={unlockPw}
            onChange={(e) => setUnlockPw(e.target.value)}
            placeholder={t("auth.security.unlockPwPh")}
            onKeyDown={(e) => e.key === "Enter" && doUnlock()}
          />
          <button className="primary xs" disabled={busy || !unlockPw} onClick={doUnlock}>
            {busy ? <Spinner size="xs" inline /> : t("auth.security.unlock")}
          </button>
        </div>
      )}

      {mode === "idle" && (
        <div className="actions">
          {!enabled && (
            <button className="xs" onClick={() => setMode("enable")}>
              {t("auth.security.enable")}
            </button>
          )}
          {enabled && (
            <>
              <button className="xs" onClick={() => setMode("change")}>
                {t("auth.security.change")}
              </button>
              {unlocked && (
                <button className="xs ghost" onClick={lockAuth}>
                  {t("auth.security.lock")}
                </button>
              )}
              <button className="xs danger ghost" onClick={() => setMode("disable")}>
                {t("auth.security.disable")}
              </button>
            </>
          )}
        </div>
      )}

      {(mode === "enable" || mode === "change") && (
        <div className="auth-sec-form">
          {mode === "change" && (
            <div className="auth-sec-row">
              <label>{t("auth.security.oldPw")}</label>
              <input
                type="password"
                value={oldPw}
                onChange={(e) => setOldPw(e.target.value)}
              />
            </div>
          )}
          <div className="auth-sec-row">
            <label>{t("auth.security.newPw")}</label>
            <input
              type="password"
              value={newPw}
              onChange={(e) => setNewPw(e.target.value)}
            />
          </div>
          <div className="auth-sec-row">
            <label>{t("auth.security.repeatPw")}</label>
            <input
              type="password"
              value={newPw2}
              onChange={(e) => setNewPw2(e.target.value)}
            />
          </div>
          <div className="hint">{t("auth.security.hint")}</div>
          {mode === "enable" && (
            <label
              className="auth-sec-row"
              style={{ cursor: "pointer", alignItems: "flex-start" }}
            >
              <input
                type="checkbox"
                checked={ackWarn}
                onChange={(e) => setAckWarn(e.target.checked)}
              />
              <span style={{ fontSize: 11, lineHeight: 1.5 }}>
                {t("auth.security.ackWarn")}
              </span>
            </label>
          )}
          <div className="actions">
            <button className="xs" onClick={reset} disabled={busy}>
              {t("common.cancel")}
            </button>
            <button
              className="primary"
              disabled={busy || !newPw || (mode === "enable" && !ackWarn)}
              onClick={doApply}
            >
              {busy ? <Spinner size="xs" inline /> : t("auth.security.apply")}
            </button>
          </div>
        </div>
      )}

      {mode === "disable" && (
        <div className="auth-sec-form">
          {hasFiles && (
            <div className="auth-sec-row">
              <label>{t("auth.security.oldPw")}</label>
              <input
                type="password"
                value={oldPw}
                onChange={(e) => setOldPw(e.target.value)}
              />
            </div>
          )}
          <div className="hint">{t("auth.security.disableHint")}</div>
          <div className="actions">
            <button className="xs" onClick={reset} disabled={busy}>
              {t("common.cancel")}
            </button>
            <button className="primary danger" disabled={busy} onClick={doApply}>
              {busy ? <Spinner size="xs" inline /> : t("auth.security.disableBtn")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
