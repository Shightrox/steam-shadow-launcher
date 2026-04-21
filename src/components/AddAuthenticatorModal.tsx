import { useEffect, useRef, useState } from "react";
import { api, type AddCreatePublic, type AddDiagnostic } from "../api/tauri";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { Spinner } from "./Spinner";
import { ErrorBox } from "./ErrorBox";
import { LoginFlowModal } from "./LoginFlowModal";

/// Wizard phases (in order):
///   login           — reuse LoginFlowModal to obtain tokens
///   diagnose        — auto-call QueryStatus + AccountPhoneStatus
///   diag-panel      — show real account state, recommend ONE primary action
///   blocker-guard   — Email Guard is OFF → cannot proceed without enabling
///   blocker-mobile  — already has a mobile authenticator → must revoke first
///   phone..persist  — actual flow steps
type Phase =
  | "login"
  | "diagnose"
  | "diag-panel"
  | "blocker-guard"
  | "blocker-mobile"
  | "phone"
  | "email-wait"
  | "sms-send"
  | "sms-verify"
  | "create"
  | "finalize"
  | "revocation"
  | "persist"
  | "done";

interface Props {
  open: boolean;
  login: string;
  onClose(): void;
  onSuccess?(): void;
}

export function AddAuthenticatorModal({ open, login, onClose, onSuccess }: Props) {
  const { t } = useI18n();
  const { refreshAuthStatus, refreshAccounts, refreshCode, toast, setAddAuthActive } = useApp();

  const [phase, setPhase] = useState<Phase>("login");
  const [loginDone, setLoginDone] = useState(false);
  const [phoneNumber, setPhoneNumber] = useState("");
  const [countryCode, setCountryCode] = useState("RU");
  const [phoneAttached, setPhoneAttached] = useState(false);
  const [sms1, setSms1] = useState("");
  const [sms2, setSms2] = useState("");
  const [tryNumber, setTryNumber] = useState(1);
  const [createRes, setCreateRes] = useState<AddCreatePublic | null>(null);
  const [diag, setDiag] = useState<AddDiagnostic | null>(null);
  const [revocation, setRevocation] = useState<string | null>(null);
  const [confirmedWritten, setConfirmedWritten] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const emailPoll = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!open) return;
    setPhase("login");
    setLoginDone(false);
    setPhoneNumber("");
    setCountryCode("RU");
    setPhoneAttached(false);
    setSms1("");
    setSms2("");
    setTryNumber(1);
    setCreateRes(null);
    setDiag(null);
    setRevocation(null);
    setConfirmedWritten(false);
    setError(null);
    setBusy(false);
  }, [open]);

  // Expose a globally-visible "wizard is mounted" flag so the title bar can
  // prompt-to-confirm before the window is killed mid-flow.
  useEffect(() => {
    setAddAuthActive(open);
    return () => setAddAuthActive(false);
  }, [open]);

  useEffect(() => {
    return () => {
      if (emailPoll.current) clearInterval(emailPoll.current);
    };
  }, []);

  if (!open) return null;

  const abort = async () => {
    if (emailPoll.current) {
      clearInterval(emailPoll.current);
      emailPoll.current = null;
    }
    try {
      await api.authAddCancel(login);
    } catch {
      /* ignore */
    }
    onClose();
  };

  const doDiagnose = async () => {
    setBusy(true);
    setError(null);
    try {
      const d = await api.authAddDiagnose(login);
      setDiag(d);
      switch (d.suggested_path) {
        case "blocker-no-guard":
          setPhase("blocker-guard");
          break;
        case "blocker-already-mobile":
          setPhase("blocker-mobile");
          break;
        default:
          setPhase("diag-panel");
      }
    } catch (e: any) {
      setError(String(e));
      setPhase("diag-panel");
    } finally {
      setBusy(false);
    }
  };

  // ── login phase: delegate to LoginFlowModal; on close it'll have seeded
  //    the AddSession registry inside Rust (put_session on PollState::Done).
  if (phase === "login") {
    return (
      <LoginFlowModal
        open={true}
        login={login}
        defaultAccountName={login}
        onClose={() => {
          if (loginDone) {
            setPhase("diagnose");
            doDiagnose();
          } else {
            onClose();
          }
        }}
        onSuccess={() => setLoginDone(true)}
      />
    );
  }

  // ── phone attach / skip
  const doSetPhone = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.authAddSetPhone(login, phoneNumber.trim(), countryCode.trim().toUpperCase());
      setPhase("email-wait");
      startEmailPoll();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const skipPhone = async () => {
    // Same as intro's "no phone" branch — kept for the in-phone-step button.
    setPhoneAttached(true);
    await doCreate();
  };

  const startEmailPoll = () => {
    if (emailPoll.current) clearInterval(emailPoll.current);
    const tick = async () => {
      try {
        const s = await api.authAddCheckEmail(login);
        if (!s.awaiting_email) {
          if (emailPoll.current) clearInterval(emailPoll.current);
          emailPoll.current = null;
          setPhase("sms-send");
          doSendSms();
        }
      } catch (e: any) {
        if (emailPoll.current) clearInterval(emailPoll.current);
        emailPoll.current = null;
        setError(String(e));
      }
    };
    emailPoll.current = setInterval(tick, 5000);
    tick();
  };

  const doSendSms = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.authAddSendSms(login);
      setPhase("sms-verify");
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doVerifyPhone = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.authAddVerifyPhone(login, sms1.trim());
      await doCreate();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doCreate = async () => {
    setBusy(true);
    setError(null);
    setPhase("create");
    try {
      const r = await api.authAddCreate(login);
      setCreateRes(r);
      // Whether the account has a phone attached or only email Guard, Steam
      // *always* sends an activation code at this point (SMS for phone, email
      // for no-phone). The user must type it in. No auto-finalize.
      setPhase("finalize");
    } catch (e: any) {
      const msg = String(e);
      if (msg.includes("ADD_AUTH_NEED_PHONE")) {
        setError(t("auth.add.needPhone"));
        setPhase("phone");
        setPhoneAttached(false);
      } else if (msg.includes("ADD_AUTH_ALREADY_HAS_AUTHENTICATOR")) {
        setError(t("auth.add.alreadyHas"));
        setPhase("blocker-mobile");
      } else if (msg.includes("ADD_AUTH_RATE_LIMIT")) {
        setError(t("auth.add.rateLimit"));
        setPhase("diag-panel");
      } else {
        setError(msg);
        setPhase("diag-panel");
      }
    } finally {
      setBusy(false);
    }
  };

  const doFinalize = async (validateSms: boolean = true) => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.authAddFinalize(
        login,
        validateSms ? sms2.trim() : "",
        tryNumber,
        validateSms,
      );
      if (r.success && r.revocation_code) {
        setRevocation(r.revocation_code);
        setPhase("revocation");
      } else if (r.want_more) {
        // Server clock drift — increment try_number and retry w/ SAME code.
        setTryNumber((n) => n + 1);
        setTimeout(() => doFinalize(validateSms), 500);
      } else if (!validateSms) {
        // No-phone attempt rejected — hint to use SMS instead.
        setError(t("auth.add.noPhoneRejected"));
      } else {
        // status 89 = SMS mismatch, status 88 = rate-limit.
        setError(t("auth.add.finalizeBadSms", { status: r.status }));
      }
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doPersist = async () => {
    setBusy(true);
    setError(null);
    setPhase("persist");
    try {
      await api.authAddPersist(login);
      await refreshAccounts();
      await refreshAuthStatus();
      await refreshCode(login);
      setPhase("done");
      toast("success", t("auth.add.success"));
      onSuccess?.();
    } catch (e: any) {
      setError(String(e));
      setPhase("revocation");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={() => !busy && abort()}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ minWidth: 480 }}
      >
        <div className="modal-title">
          {t("auth.add.title")} → {login}
        </div>
        <div className="modal-body">
          {phase === "diagnose" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.add.diagnosing")}</div>
            </div>
          )}

          {phase === "diag-panel" && !diag && (
            <>
              <div className="auth-warn" style={{ fontSize: 14 }}>
                ⚠ {t("auth.add.diagFailedTitle")}
              </div>
              <div className="hint" style={{ marginTop: 8 }}>
                {t("auth.add.diagFailedHint")}
              </div>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button
                  className="primary"
                  disabled={busy}
                  onClick={() => doDiagnose()}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.btnRecheck")}
                </button>
                <button className="xs ghost" onClick={abort}>
                  {t("common.cancel")}
                </button>
              </div>
            </>
          )}

          {phase === "diag-panel" && diag && (
            <>
              <div className="hint" style={{ marginBottom: 8 }}>
                {t("auth.add.diagPanelTitle")}
              </div>
              <div className="diag-grid">
                <DiagRow
                  label={t("auth.add.diagGuard")}
                  ok={diag.guard !== "none"}
                  value={
                    diag.guard === "email"
                      ? t("auth.add.guardEmail")
                      : diag.guard === "mobile"
                        ? t("auth.add.guardMobile")
                        : t("auth.add.guardNone")
                  }
                />
                <DiagRow
                  label={t("auth.add.diagPhone")}
                  ok={diag.phone_attached}
                  neutral={!diag.phone_attached && diag.guard === "email"}
                  value={
                    diag.phone_attached
                      ? diag.phone_hint || t("auth.add.phoneAttached")
                      : t("auth.add.phoneNotAttached")
                  }
                />
              </div>
              <div className="hint" style={{ marginTop: 12, opacity: 0.8 }}>
                {diag.suggested_path === "no-phone-fast"
                  ? t("auth.add.recoNoPhone")
                  : t("auth.add.recoPhone")}
              </div>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions" style={{ flexDirection: "column", gap: 8 }}>
                {diag.suggested_path === "no-phone-fast" ? (
                  <>
                    <button
                      className="primary"
                      style={{ width: "100%" }}
                      onClick={async () => {
                        setPhoneAttached(true);
                        await doCreate();
                      }}
                    >
                      ⚡ {t("auth.add.btnAttachNoPhone")}
                    </button>
                    <button
                      className="xs"
                      style={{ width: "100%" }}
                      onClick={() => setPhase("phone")}
                    >
                      {t("auth.add.btnAttachPhoneInstead")}
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      className="primary"
                      style={{ width: "100%" }}
                      onClick={async () => {
                        setPhoneAttached(true);
                        await doCreate();
                      }}
                    >
                      📱 {t("auth.add.btnContinueWithPhone")}
                    </button>
                  </>
                )}
                <button className="xs ghost" onClick={abort}>
                  {t("common.cancel")}
                </button>
              </div>
            </>
          )}

          {phase === "blocker-guard" && (
            <>
              <div className="auth-warn" style={{ fontSize: 14 }}>
                ⛔ {t("auth.add.blockGuardTitle")}
              </div>
              <div className="hint" style={{ marginTop: 8 }}>
                {t("auth.add.blockGuardBody")}
              </div>
              <ol className="diag-steps">
                <li>{t("auth.add.blockGuardStep1")}</li>
                <li>{t("auth.add.blockGuardStep2")}</li>
                <li>{t("auth.add.blockGuardStep3")}</li>
              </ol>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button
                  className="xs"
                  onClick={() => api.openUrl("steam://settings/account")}
                >
                  ↗ {t("auth.add.btnOpenSteam")}
                </button>
                <button
                  className="xs"
                  disabled={busy}
                  onClick={() => doDiagnose()}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.btnRecheck")}
                </button>
                <button className="xs ghost" onClick={abort}>
                  {t("common.cancel")}
                </button>
              </div>
            </>
          )}

          {phase === "blocker-mobile" && (
            <>
              <div className="auth-warn" style={{ fontSize: 14 }}>
                ⛔ {t("auth.add.blockMobileTitle")}
              </div>
              <div className="hint" style={{ marginTop: 8 }}>
                {t("auth.add.blockMobileBody")}
              </div>
              <ol className="diag-steps">
                <li>{t("auth.add.blockMobileStep1")}</li>
                <li>{t("auth.add.blockMobileStep2")}</li>
                <li>{t("auth.add.blockMobileStep3")}</li>
              </ol>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button
                  className="xs"
                  onClick={() =>
                    api.openUrl("https://store.steampowered.com/twofactor/manage")
                  }
                >
                  ↗ {t("auth.add.btnOpenSteamWeb")}
                </button>
                <button
                  className="xs"
                  disabled={busy}
                  onClick={() => doDiagnose()}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.btnRecheck")}
                </button>
                <button className="xs ghost" onClick={abort}>
                  {t("common.cancel")}
                </button>
              </div>
            </>
          )}

          {phase === "phone" && (
            <>
              <div className="hint">{t("auth.add.phoneHint")}</div>
              <div className="field">
                <label>{t("auth.add.countryCode")}</label>
                <input
                  value={countryCode}
                  maxLength={2}
                  onChange={(e) =>
                    setCountryCode(e.target.value.toUpperCase().slice(0, 2))
                  }
                  placeholder="RU"
                />
              </div>
              <div className="field">
                <label>{t("auth.add.phone")}</label>
                <input
                  value={phoneNumber}
                  onChange={(e) => setPhoneNumber(e.target.value)}
                  placeholder="+79001234567"
                  autoFocus
                />
              </div>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions" style={{ marginTop: 12 }}>
                <button className="xs" onClick={abort} disabled={busy}>
                  {t("common.cancel")}
                </button>
                <button className="xs ghost" onClick={skipPhone} disabled={busy}>
                  {t("auth.add.skipPhone")}
                </button>
                <button
                  className="primary"
                  onClick={doSetPhone}
                  disabled={busy || !phoneNumber.trim() || !countryCode.trim()}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.attachPhone")}
                </button>
              </div>
            </>
          )}

          {phase === "email-wait" && (
            <>
              <div className="auth-loading">
                <Spinner size="md" />
                <div>{t("auth.add.emailWait")}</div>
                <div className="hint">{t("auth.add.emailWaitHint")}</div>
              </div>
              <div className="modal-actions">
                <button className="xs" onClick={abort}>
                  {t("common.cancel")}
                </button>
              </div>
            </>
          )}

          {phase === "sms-send" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.add.sendingSms")}</div>
            </div>
          )}

          {phase === "sms-verify" && (
            <>
              <div className="hint">{t("auth.add.smsVerifyHint")}</div>
              <div className="field">
                <label>{t("auth.add.smsCode")}</label>
                <input
                  value={sms1}
                  onChange={(e) => setSms1(e.target.value.replace(/\s/g, ""))}
                  placeholder="12345"
                  autoFocus
                  maxLength={10}
                  style={{
                    fontFamily: "var(--pixel)",
                    fontSize: 22,
                    letterSpacing: 4,
                  }}
                  onKeyDown={(e) => e.key === "Enter" && doVerifyPhone()}
                />
              </div>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button className="xs" onClick={abort} disabled={busy}>
                  {t("common.cancel")}
                </button>
                <button
                  className="xs ghost"
                  onClick={doSendSms}
                  disabled={busy}
                  title={t("auth.add.resendSms")}
                >
                  ↻ {t("auth.add.resendSms")}
                </button>
                <button
                  className="primary"
                  disabled={busy || !sms1}
                  onClick={doVerifyPhone}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.next")}
                </button>
              </div>
            </>
          )}

          {phase === "create" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.add.creating")}</div>
            </div>
          )}

          {phase === "finalize" && (
            <>
              <div className="hint">
                {createRes?.phone_number_hint
                  ? t("auth.add.finalizeHint", {
                      hint: createRes.phone_number_hint,
                    })
                  : t("auth.add.finalizeHintEmail")}
              </div>
              <div className="field">
                <label>
                  {createRes?.phone_number_hint
                    ? t("auth.add.smsCodeActivation")
                    : t("auth.add.emailCodeActivation")}
                </label>
                <input
                  value={sms2}
                  onChange={(e) => setSms2(e.target.value.replace(/\s/g, ""))}
                  placeholder={createRes?.phone_number_hint ? "12345" : "ABCDE"}
                  autoFocus
                  maxLength={10}
                  style={{
                    fontFamily: "var(--pixel)",
                    fontSize: 22,
                    letterSpacing: 4,
                  }}
                  onKeyDown={(e) => e.key === "Enter" && !busy && doFinalize(true)}
                />
              </div>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button className="xs" onClick={abort} disabled={busy}>
                  {t("common.cancel")}
                </button>
                <button
                  className="primary"
                  disabled={busy || !sms2}
                  onClick={() => doFinalize(true)}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.finalize")}
                </button>
              </div>
            </>
          )}

          {phase === "revocation" && revocation && (
            <>
              <div className="auth-warn" style={{ fontSize: 14 }}>
                ⚠ {t("auth.add.revocationTitle")}
              </div>
              <div
                style={{
                  fontFamily: "var(--pixel)",
                  fontSize: 36,
                  textAlign: "center",
                  padding: "20px 0",
                  color: "var(--danger)",
                  letterSpacing: 6,
                }}
              >
                {revocation}
              </div>
              <div className="hint">{t("auth.add.revocationHint")}</div>
              <label
                className="auth-sec-row"
                style={{ marginTop: 12, cursor: "pointer" }}
              >
                <input
                  type="checkbox"
                  checked={confirmedWritten}
                  onChange={(e) => setConfirmedWritten(e.target.checked)}
                />
                <span>{t("auth.add.revocationConfirm")}</span>
              </label>
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button
                  className="xs ghost"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(revocation);
                      toast("success", t("auth.copied"));
                    } catch (e: any) {
                      toast("error", String(e));
                    }
                  }}
                >
                  ⎘ {t("auth.copy")}
                </button>
                <button
                  className="primary"
                  disabled={!confirmedWritten || busy}
                  onClick={doPersist}
                >
                  {busy ? <Spinner size="xs" inline /> : t("auth.add.savePersist")}
                </button>
              </div>
            </>
          )}

          {phase === "persist" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.add.persisting")}</div>
            </div>
          )}

          {phase === "done" && (
            <div className="auth-done">
              <div style={{ fontSize: 48, color: "var(--accent)" }}>✓</div>
              <div>{t("auth.add.success")}</div>
              <div className="modal-actions">
                <button className="primary" onClick={onClose}>
                  {t("common.close")}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function DiagRow({
  label,
  ok,
  neutral,
  value,
}: {
  label: string;
  ok: boolean;
  neutral?: boolean;
  value: string;
}) {
  const icon = ok ? "🟢" : neutral ? "⚪" : "🔴";
  return (
    <div className="diag-row">
      <span className="diag-icon">{icon}</span>
      <span className="diag-label">{label}</span>
      <span className="diag-value">{value}</span>
    </div>
  );
}
