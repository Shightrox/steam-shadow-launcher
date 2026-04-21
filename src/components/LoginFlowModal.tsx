import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { useApp } from "../state/store";
import {
  api,
  type BeginOutcome,
  type PollState,
} from "../api/tauri";
import { Spinner } from "./Spinner";
import { ErrorBox } from "./ErrorBox";

type Phase =
  | "credentials"
  | "beginning"
  | "code"
  | "polling"
  | "done"
  | "failed";

interface Props {
  open: boolean;
  /** Shadow-account login that the resulting session will be stored under. */
  login: string;
  /** Initial account_name to pre-fill (usually the shadow login). */
  defaultAccountName?: string;
  onClose(): void;
  /** Fired exactly once when poll returns state=Done and tokens were saved. */
  onSuccess?(): void;
}

// EAuthSessionGuardType (Valve enum):
//   1 = None, 2 = EmailCode, 3 = DeviceCode,
//   4 = DeviceConfirmation, 5 = EmailConfirmation,
//   6 = MachineToken, 7 = LegacyMachineAuth
const GUARD_TYPE_NONE = 1;
const GUARD_TYPE_EMAIL = 2;
const GUARD_TYPE_DEVICE = 3;
const GUARD_TYPE_DEVICE_CONFIRM = 4;
const GUARD_TYPE_EMAIL_CONFIRM = 5;

function guardMethodLabel(
  t: (k: string, p?: Record<string, string>) => string,
  kind: number,
): string {
  switch (kind) {
    case GUARD_TYPE_EMAIL:
      return t("auth.login.guardEmail");
    case GUARD_TYPE_DEVICE:
      return t("auth.login.guardDevice");
    case GUARD_TYPE_NONE:
      return t("auth.login.guardNone");
    case GUARD_TYPE_DEVICE_CONFIRM:
      return t("auth.login.guardMobileApp");
    case GUARD_TYPE_EMAIL_CONFIRM:
      return t("auth.login.guardEmail");
    default:
      return t("auth.login.guardOther");
  }
}

export function LoginFlowModal({
  open,
  login,
  defaultAccountName,
  onClose,
  onSuccess,
}: Props) {
  const { t } = useI18n();
  const { refreshAuthStatus, refreshAccounts, refreshCode, toast } = useApp();
  const [phase, setPhase] = useState<Phase>("credentials");
  const [accountName, setAccountName] = useState(defaultAccountName ?? login ?? "");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [codeType, setCodeType] = useState(GUARD_TYPE_DEVICE);
  const [begin, setBegin] = useState<BeginOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  // Reset when reopened.
  useEffect(() => {
    if (!open) return;
    setPhase("credentials");
    setAccountName(defaultAccountName ?? login ?? "");
    setPassword("");
    setCode("");
    setBegin(null);
    setError(null);
    setBusy(false);
  }, [open, login, defaultAccountName]);

  // Cleanup poll timer on unmount / close.
  useEffect(() => {
    if (!open && pollTimer.current) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, [open]);

  if (!open) return null;

  const stopPolling = () => {
    if (pollTimer.current) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  };

  const startPolling = (b: BeginOutcome) => {
    stopPolling();
    const interval = Math.max(2, Math.floor(b.interval)) * 1000;
    const tick = async () => {
      try {
        const state: PollState = await api.authLoginPoll(
          login,
          b.clientId,
          b.requestId,
          b.allowedConfirmations.map((c) => c.confirmation_type),
        );
        if (state.state === "Done") {
          stopPolling();
          setPhase("done");
          await refreshAccounts();
          await refreshAuthStatus();
          await refreshCode(login);
          toast("success", t("auth.login.success"));
          onSuccess?.();
        } else if (state.state === "NeedsCode") {
          stopPolling();
          // Previous code is stale — Steam rejected it. Clear input so the
          // user can't accidentally resubmit without noticing.
          setCode("");
          setPhase("code");
          setError(t("auth.login.needsCode"));
        } else if (state.state === "Failed") {
          stopPolling();
          setPhase("failed");
          setError(state.reason || "Auth session expired");
        }
      } catch (e: any) {
        stopPolling();
        setPhase("failed");
        setError(String(e));
      }
    };
    pollTimer.current = setInterval(tick, interval);
    // First poll immediately for snappier UX.
    tick();
  };

  const doBegin = async () => {
    setBusy(true);
    setError(null);
    setPhase("beginning");
    try {
      const b = await api.authLoginBegin(accountName.trim(), password);
      setBegin(b);
      // Decide whether the user must type a code or just click a link.
      //
      // confirmation_type:
      //   2 = None             → no code, go to polling
      //   3 = EmailCode        → user types 5-char code from email
      //   4 = DeviceCode       → user types TOTP from SDA
      //   5 = DeviceConfirmation → user taps "approve" in mobile app, we poll
      //   6 = EmailConfirmation  → user clicks link in email, we poll
      //
      // Only 3 and 4 require the "enter code" screen. 5/6 are click-and-poll.
      const needsCode = b.allowedConfirmations.some(
        (c) => c.confirmation_type === GUARD_TYPE_DEVICE
            || c.confirmation_type === GUARD_TYPE_EMAIL,
      );
      if (!needsCode) {
        setPhase("polling");
        startPolling(b);
        return;
      }
      // Pick a reasonable default guard type from what Steam offers.
      const hasDevice = b.allowedConfirmations.some(
        (c) => c.confirmation_type === GUARD_TYPE_DEVICE,
      );
      const hasEmail = b.allowedConfirmations.some(
        (c) => c.confirmation_type === GUARD_TYPE_EMAIL,
      );
      if (hasDevice) setCodeType(GUARD_TYPE_DEVICE);
      else if (hasEmail) setCodeType(GUARD_TYPE_EMAIL);
      setPhase("code");
    } catch (e: any) {
      setPhase("credentials");
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doSubmitCode = async () => {
    if (!begin) return;
    if (!code.trim()) {
      setError(t("auth.login.codeRequired"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.authLoginSubmitCode(begin.clientId, begin.steamId, code.trim(), codeType);
      setPhase("polling");
      startPolling(begin);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const close = () => {
    stopPolling();
    onClose();
  };

  return (
    <div className="modal-backdrop" onClick={() => !busy && close()}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ minWidth: 420 }}>
        <div className="modal-title">
          {t("auth.login.title")} → {login}
        </div>
        <div className="modal-body">
          {phase === "credentials" && (
            <>
              <div className="field">
                <label>{t("auth.login.accountName")}</label>
                <input
                  value={accountName}
                  onChange={(e) => setAccountName(e.target.value)}
                  autoFocus
                  autoCapitalize="off"
                  autoComplete="off"
                  spellCheck={false}
                />
              </div>
              <div className="field">
                <label>{t("auth.login.password")}</label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && accountName && password) doBegin();
                  }}
                />
              </div>
              {error && <ErrorBox message={error} />}
              <div className="hint">{t("auth.login.credentialsHint")}</div>
            </>
          )}

          {phase === "beginning" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.login.beginning")}</div>
            </div>
          )}

          {phase === "code" && begin && (
            <>
              <div className="auth-login-info">
                {t("auth.login.steamId")}: <span className="dim">{begin.steamId}</span>
              </div>
              {begin.allowedConfirmations.length > 0 && (
                <div className="field">
                  <label>{t("auth.login.guardMethod")}</label>
                  <div className="guard-methods">
                    {begin.allowedConfirmations
                      .filter((c) => [GUARD_TYPE_DEVICE, GUARD_TYPE_EMAIL, GUARD_TYPE_NONE, GUARD_TYPE_DEVICE_CONFIRM].includes(c.confirmation_type))
                      .map((c) => (
                        <button
                          key={c.confirmation_type}
                          className={`xs ${codeType === c.confirmation_type ? "active" : ""}`}
                          onClick={() => setCodeType(c.confirmation_type)}
                          disabled={c.confirmation_type === GUARD_TYPE_NONE}
                        >
                          {guardMethodLabel(t, c.confirmation_type)}
                          {c.associated_message && (
                            <span className="dim"> · {c.associated_message}</span>
                          )}
                        </button>
                      ))}
                  </div>
                </div>
              )}
              <div className="field">
                <label>{t("auth.login.codeLabel")}</label>
                <input
                  value={code}
                  onChange={(e) => setCode(e.target.value.toUpperCase())}
                  placeholder="ABCDE"
                  maxLength={7}
                  autoFocus
                  style={{ fontFamily: "var(--pixel)", fontSize: 22, letterSpacing: 4 }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") doSubmitCode();
                  }}
                />
              </div>
              {error && <ErrorBox message={error} />}
              <div className="hint">{t("auth.login.codeHint")}</div>
            </>
          )}

          {phase === "polling" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("auth.login.polling")}</div>
              <div className="hint">{t("auth.login.pollingHint")}</div>
            </div>
          )}

          {phase === "done" && (
            <div className="auth-done">
              <div style={{ fontSize: 48, color: "var(--accent)" }}>✓</div>
              <div>{t("auth.login.success")}</div>
            </div>
          )}

          {phase === "failed" && (
            <div>
              <div style={{ fontSize: 32, color: "var(--danger)", textAlign: "center" }}>✕</div>
              <ErrorBox message={error ?? "Failed"} />
            </div>
          )}
        </div>

        <div className="modal-actions">
          {phase === "credentials" && (
            <>
              <button className="xs" onClick={close} disabled={busy}>
                {t("common.cancel")}
              </button>
              <button
                className="primary"
                disabled={busy || !accountName.trim() || !password}
                onClick={doBegin}
              >
                {busy ? <Spinner size="xs" inline /> : t("auth.login.next")}
              </button>
            </>
          )}
          {phase === "code" && (
            <>
              <button className="xs" onClick={close} disabled={busy}>
                {t("common.cancel")}
              </button>
              <button
                className="primary"
                disabled={busy}
                onClick={doSubmitCode}
              >
                {busy ? <Spinner size="xs" inline /> : t("auth.login.submit")}
              </button>
            </>
          )}
          {(phase === "polling" || phase === "beginning") && (
            <button className="xs" onClick={close}>
              {t("common.cancel")}
            </button>
          )}
          {phase === "done" && (
            <button className="primary" onClick={close}>
              {t("common.close")}
            </button>
          )}
          {phase === "failed" && (
            <>
              <button className="xs" onClick={close}>
                {t("common.close")}
              </button>
              <button className="primary" onClick={() => setPhase("credentials")}>
                {t("common.retry")}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
