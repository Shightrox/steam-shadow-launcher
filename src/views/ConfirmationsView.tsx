import { useEffect, useState } from "react";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { ConfirmationsSection } from "../components/ConfirmationsSection";
import { Icon } from "../components/Icon";

export function ConfirmationsView({ initialLogin, onManage }: { initialLogin?: string | null; onManage(login?: string): void }) {
  const { t } = useI18n();
  const accounts = useApp(s => s.accounts);
  const statuses = useApp(s => s.authStatus);
  const locked = useApp(s => !!s.authLock?.enabled && !s.authLock.unlocked);
  const epoch = useApp(s => s.workspaceEpoch);
  const [filter, setFilter] = useState(initialLogin ?? "");
  useEffect(() => setFilter(initialLogin ?? ""), [initialLogin]);
  const eligible = accounts.filter(a => a.hasAuthenticator || statuses[a.login]?.hasAuthenticator);
  const accountFilter = eligible.some(a => a.login === filter) ? filter : "";
  const visible = eligible.filter(a => !accountFilter || a.login === accountFilter);
  return <div className="confirmations-view">
    <div className="page-heading"><div><h1>{t("auth.confirmations")}</h1><p>{t("design.confirmationsHint")}</p></div></div>
    {locked ? <div className="empty-state"><Icon name="shield" size={32} /><h2>{t("auth.locked")}</h2><p>{t("auth.lockedHint")}</p><button className="primary" onClick={() => onManage()}>{t("auth.security.unlock")}</button></div>
      : eligible.length === 0 ? <div className="empty-state"><Icon name="shield" size={32} /><h2>{t("auth.noAuthYet")}</h2><p>{t("auth.noAuthHint")}</p><button onClick={() => onManage()}>{t("design.manageAccess")}</button></div>
      : <><label className="account-filter">{t("design.accountFilter")}<select value={accountFilter} onChange={e => setFilter(e.target.value)}><option value="">{t("design.allAccounts")}</option>{eligible.map(a => <option key={a.login} value={a.login}>{a.displayName || a.login} · @{a.login}</option>)}</select></label>
        <div className="confirmation-groups">{visible.map(a => <ConfirmationsSection key={`${epoch}:${a.login}`} login={a.login} title={`${a.displayName || a.login} · @${a.login}`} onManage={() => onManage(a.login)} />)}</div>
      </>}
  </div>;
}
