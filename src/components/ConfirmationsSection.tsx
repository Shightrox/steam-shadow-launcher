import { useEffect, useState } from "react";
import { api, type Confirmation, type TradeDetails, type TradeItem } from "../api/tauri";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { Spinner } from "./Spinner";
import { ErrorBox } from "./ErrorBox";
import { ItemImage } from "./ItemImage";
import { Icon } from "./Icon";

function typeLabel(t: (key: string) => string, kind: number) {
  return t(({ 2: "auth.typeTrade", 3: "auth.typeMarket", 5: "auth.typePhone", 6: "auth.typeAccountRecovery", 9: "auth.typeApiKey" } as Record<number, string>)[kind] ?? "auth.typeOther");
}

function TradeItems({ items, title }: { items: TradeItem[]; title: string }) {
  const { t } = useI18n();
  return <div className="trade-side"><h4>{title}<span>{items.reduce((n, item) => n + BigInt(/^\d+$/.test(item.amount) ? item.amount : "0"), BigInt(0)).toString()}</span></h4>
    {items.length === 0 ? <p className="hint">{t("design.noItems")}</p> : <div className="trade-items">{items.map((item, i) => <div className="trade-item" key={`${item.appId}:${item.assetId}:${i}`}>
      <ItemImage src={item.icon} name={item.name || t("design.unknownItem")} />
      <div><span>{item.name || t("design.unknownItem")}</span><small>{item.amount !== "1" ? `× ${item.amount} · ` : ""}{item.name ? `App ${item.appId}` : `App ${item.appId} · #${item.assetId}`}</small></div>
    </div>)}</div>}
  </div>;
}

function TradeContents({ login, confirmation }: { login: string; confirmation: Confirmation }) {
  const { t } = useI18n();
  const epoch = useApp(s => s.workspaceEpoch);
  const locked = useApp(s => !!s.authLock?.enabled && !s.authLock.unlocked);
  const [details, setDetails] = useState<TradeDetails | null>(null);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let alive = true;
    setDetails(null); setError(false);
    if (!locked) api.authConfirmationDetails(login, confirmation.id).then(result => {
      if (alive && useApp.getState().workspaceEpoch === epoch) setDetails(result);
    }).catch(() => { if (alive && useApp.getState().workspaceEpoch === epoch) setError(true); });
    return () => { alive = false; };
  }, [login, confirmation.id, confirmation.nonce, epoch, locked, attempt]);
  if (locked) return null;
  if (error) return <div className="trade-unavailable"><span>{t("design.detailsUnavailable")}</span><button className="xs" onClick={() => setAttempt(x => x + 1)}>{t("common.retry")}</button><button className="xs ghost" onClick={() => api.openUrl(`https://steamcommunity.com/tradeoffer/${encodeURIComponent(confirmation.creator_id)}/`).catch(e => useApp.getState().toast("error", String(e)))}>{t("design.openTrade")}</button></div>;
  if (!details) return <div className="trade-loading" role="status"><Spinner size="xs" inline /> {t("design.loadingItems")}</div>;
  return <div className="trade-contents">
    <div className="trade-columns"><TradeItems items={details.giving} title={t("design.giving")} /><TradeItems items={details.receiving} title={t("design.receiving")} /></div>
    <div className="trade-partner">{t("design.partnerId")}: <span>{details.partnerSteamId}</span></div>
  </div>;
}

function ConfirmationRow({ login, item, checked, disabled, onSelect }: { login: string; item: Confirmation; checked: boolean; disabled: boolean; onSelect(value: boolean): void }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  return <article className={`confirmation-card${checked ? " checked" : ""}`}>
    <div className="confirmation-summary">
      <input type="checkbox" aria-label={`${t("design.select")} ${item.headline}`} checked={checked} disabled={disabled} onChange={e => onSelect(e.target.checked)} />
      <ItemImage src={item.icon} avatar={item.type === 2} name={item.type === 2 ? item.headline : item.summary?.[0] || item.headline} />
      <div className="auth-conf-meta"><div className="auth-conf-type">{typeLabel(t, item.type)}</div><div className="auth-conf-headline">{item.headline}</div>
        {item.summary?.map((line, i) => <div key={i} className="auth-conf-summary">{line}</div>)}
      </div>
      {item.type === 2 && <button className="xs ghost" aria-expanded={expanded} onClick={() => setExpanded(v => !v)}>{expanded ? t("design.hideItems") : t("design.showItems")}</button>}
    </div>
    {item.type === 2 && expanded && <TradeContents login={login} confirmation={item} />}
  </article>;
}

export function ConfirmationsSection({ login, title, onManage }: { login: string; title?: string; onManage?(): void }) {
  const { t } = useI18n();
  const confirmations = useApp(s => s.confirmations[login]);
  const loading = useApp(s => !!s.confLoading[login]);
  const error = useApp(s => s.confErrors[login]);
  const epoch = useApp(s => s.workspaceEpoch);
  const locked = useApp(s => !!s.authLock?.enabled && !s.authLock.unlocked);
  const refresh = useApp(s => s.refreshConfirmations);
  const respond = useApp(s => s.respondConfirmations);
  const [selection, setSelection] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (!locked) void refresh(login); setSelection({}); }, [login, epoch, locked]);
  const list = locked ? [] : confirmations ?? [];
  // Retain vanished selected IDs; never silently turn a stale selection into All.
  const selected = Object.keys(selection).filter(id => selection[id]);
  const doRespond = async (op: "allow" | "reject") => {
    const ids = selected.length ? selected : list.map(c => c.id);
    if (!ids.length || busy || loading || error || locked) return;
    setBusy(true);
    try { if (await respond(login, ids, op)) setSelection({}); }
    finally { setBusy(false); }
  };
  const readableError = error?.includes("AUTH_PASSWORD_UNAVAILABLE") ? t("auth.login.passwordUnavailable")
    : error?.includes("AUTH_AUTO_LOGIN_COOLDOWN") ? t("auth.login.autoCooldown")
    : error && /AUTH_(AUTO_LOGIN_NEEDS_INPUT|LOGIN_REJECTED|GUARD_REJECTED)/.test(error) ? t("auth.login.autoNeedsInput") : error;
  return <section className="auth-conf" aria-label={title || t("auth.confirmations")}>
    <div className="auth-conf-head"><div className="auth-conf-title">{title || t("auth.confirmations")}<span className="auth-conf-count">{list.length}</span></div><div className="inline-actions">
      {onManage && <button className="xs ghost" onClick={onManage} title={t("design.manageAccess")}><Icon name="shield" />{t("design.access")}</button>}
      <button className="xs icon-button" aria-label={t("common.refresh")} disabled={loading || busy || locked} onClick={async () => { await refresh(login); setSelection({}); }}>{loading ? <Spinner size="xs" inline /> : <Icon name="refresh" />}</button>
    </div></div>
    {readableError && <ErrorBox message={`${t("auth.confirmations.failed")} ${readableError}`} />}
    {list.length === 0 ? <div className="auth-conf-empty">{locked ? t("auth.locked") : loading ? <Spinner size="sm" /> : !error && <><Icon name="check" />{t("auth.confirmations.empty")}</>}</div> : <>
      <div className="auth-conf-list">{list.map(item => <ConfirmationRow key={`${epoch}:${login}:${item.id}:${item.nonce}`} login={login} item={item} checked={!!selection[item.id]} disabled={busy || loading || !!error} onSelect={value => setSelection(s => ({ ...s, [item.id]: value }))} />)}</div>
      <div className="auth-conf-actions"><span className="hint">{selected.length > 0 ? t("design.selected", { count: selected.length }) : t("design.allCount", { count: list.length })}</span><div className="spacer" /><button className="xs danger ghost" disabled={busy || loading || !!error} onClick={() => doRespond("reject")}>{selected.length ? t("auth.rejectSelected") : t("auth.rejectAll")}</button><button className="primary" disabled={busy || loading || !!error} onClick={() => doRespond("allow")}>{busy ? <Spinner size="xs" inline /> : <Icon name="check" />}{selected.length ? t("auth.allowSelected") : t("auth.allowAll")}</button></div>
    </>}
  </section>;
}
