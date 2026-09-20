import { useEffect } from "react";
import { type Account } from "../api/tauri";
import { useGuardCode, copyGuardCode } from "../state/guardCode";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { Icon } from "./Icon";

export function AccountAccess({ account, onManage, onConfirmations }: { account: Account; onManage(login: string): void; onConfirmations(): void }) {
  const { t } = useI18n();
  const status = useApp(s => s.authStatus[account.login]);
  const session = useApp(s => s.sessionStates[account.login]);
  const locked = useApp(s => !!s.authLock?.enabled && !s.authLock.unlocked);
  const count = useApp(s => s.confirmations[account.login]?.length ?? 0);
  const hasAuth = account.hasAuthenticator || status?.hasAuthenticator;
  const { code, remaining } = useGuardCode(hasAuth ? account.login : null);
  useEffect(() => { if (hasAuth && !locked) void useApp.getState().refreshSessionState(account.login); }, [account.login, hasAuth, locked]);
  const needsLogin = session === "needs_relogin" || session === "no_session";
  return <section className="account-access">
    <div className="access-heading"><div><span className="eyebrow">{t("design.selectedAccount")}</span><h2>{account.displayName || account.login}<small>@{account.login}</small></h2></div><button className="xs ghost" onClick={() => onManage(account.login)}>{t("design.manageAccess")}<Icon name="arrow" size={15} /></button></div>
    <div className="access-columns"><div className="access-guard"><div className="access-label"><Icon name="shield" />Steam Guard</div>
      {hasAuth && !locked ? <><div className="access-code-row"><span className={`access-code${remaining <= 5 ? " expiring" : ""}`}>{code?.code ?? "·····"}</span><span className="hint">{remaining}{t("design.seconds")}</span><button className="xs icon-button" disabled={!code} aria-label={t("auth.copy")} onClick={async () => { try { await copyGuardCode(account.login); useApp.getState().toast("success", t("auth.copied")); } catch (e) { useApp.getState().toast("error", String(e)); } }}><Icon name="copy" /></button></div><div className="guard-progress"><span style={{ width: `${remaining / 30 * 100}%` }} /></div></>
        : <button className="xs" onClick={() => onManage(account.login)}>{locked ? t("auth.security.unlock") : t("auth.add.title")}</button>}
    </div><div className="access-session"><div className="access-label"><span className={`state-dot${needsLogin ? " warn" : ""}`} />{t("design.session")}</div>
      <strong>{!hasAuth ? t("auth.noAuthShort") : locked ? t("auth.locked") : session === undefined ? t("common.booting") : needsLogin ? t("auth.session.reloginAction") : session === "refreshable" ? t("auth.session.recoverTitle") : t("design.sessionReady")}</strong>
      <span className="hint">{status?.hasSavedPassword ? t(`auth.auto.${status.autoLogin?.state ?? "unavailable"}`) : t("design.sessionHint")}</span>
    </div><button className="access-confirmations" onClick={onConfirmations}><Icon name="check" /><strong>{t("auth.confirmations")}</strong><span>{count > 0 ? t("design.pendingCount", { count }) : t("design.checkConfirmations")}</span><Icon name="arrow" size={16} /></button></div>
  </section>;
}
