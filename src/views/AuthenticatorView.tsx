import { ConfirmationsSection } from "../components/ConfirmationsSection";
import { Icon } from "../components/Icon";
import { ContextMenu } from "../components/ContextMenu";
import { useGuardCode, copyGuardCode } from "../state/guardCode";
import { useEffect, useMemo, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { api, pickFile, pickSaveFile } from "../api/tauri";
import { Spinner } from "../components/Spinner";
import { LoginFlowModal } from "../components/LoginFlowModal";
import { AddAuthenticatorModal } from "../components/AddAuthenticatorModal";
import { AddAccountModal } from "../components/AddAccountModal";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ErrorBox } from "../components/ErrorBox";

export function AuthenticatorView({ initialLogin }: { initialLogin?: string | null }) {
  const { t } = useI18n();
  const {
    accounts,
    authStatus,
    authLock,
    sessionStates,
    importMafile,
    exportMafile,
    removeAuthenticator,
    refreshCode,
    refreshAuthStatus,
    refreshSessionState,
    refreshAccessToken,
    refreshConfirmations,
    unlockAuth,
    add,
    toast,
  } = useApp();

  const withAuth = useMemo(
    () => accounts.filter((a) => a.hasAuthenticator || authStatus[a.login]?.hasAuthenticator),
    [accounts, authStatus],
  );
  const without = useMemo(
    () => accounts.filter((a) => !(a.hasAuthenticator || authStatus[a.login]?.hasAuthenticator)),
    [accounts, authStatus],
  );
  /** All accounts unified for the rail; "kind" tells the panel which view to render. */
  const allRail = useMemo(
    () => [
      ...withAuth.map((a) => ({ a, kind: "with" as const })),
      ...without.map((a) => ({ a, kind: "without" as const })),
    ],
    [withAuth, without],
  );

  const [selected, setSelected] = useState<string | null>(initialLogin ?? null);
  useEffect(() => { if (initialLogin) setSelected(initialLogin); }, [initialLogin]);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [importFor, setImportFor] = useState<string | null>(null);
  const [importPath, setImportPath] = useState("");
  const [importJson, setImportJson] = useState("");
  const [importBusy, setImportBusy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loginFor, setLoginFor] = useState<string | null>(null);
  const [addFor, setAddFor] = useState<string | null>(null);
  const [addAccountOpen, setAddAccountOpen] = useState(false);
  const [unlockPw, setUnlockPw] = useState("");
  const [unlockBusy, setUnlockBusy] = useState(false);
  const [removeAuthFor, setRemoveAuthFor] = useState<string | null>(null);
  const [removeAuthBusy, setRemoveAuthBusy] = useState(false);
  const [refreshBusy, setRefreshBusy] = useState(false);

  const locked = !!authLock?.enabled && !authLock.unlocked;

  // Keep a sane selection — pick from the unified list so accounts without
  // an authenticator can also be selected (so the user can attach one).
  useEffect(() => {
    if (!selected && allRail.length) setSelected(allRail[0].a.login);
    if (selected && !allRail.find(({ a }) => a.login === selected) && allRail.length) {
      setSelected(allRail[0].a.login);
    }
    if (!allRail.length && selected) setSelected(null);
  }, [allRail, selected]);

  const { code, remaining } = useGuardCode(withAuth.some(a => a.login === selected) ? selected : null);
  useEffect(() => { if (selected) void refreshSessionState(selected); }, [selected]);
  const active = selected ? accounts.find((a) => a.login === selected) ?? null : null;

  const doImport = async () => {
    if (!importFor) return;
    const src = importPath.trim() || importJson.trim();
    if (!src) {
      toast("error", t("auth.importHint"));
      return;
    }
    setImportBusy(true);
    try {
      // Peek at inline JSON body to detect SDA-encrypted form.
      // (If user passed a file path, the backend will surface
      //  `MAFILE_NEEDS_PASSWORD` on encrypted content and we prompt then.)
      let isEnc = false;
      const text = importJson.trim();
      if (text) {
        try {
          const v = JSON.parse(text);
          isEnc = v?.Encrypted === true || !!v?.encryption_iv;
        } catch {
          /* not valid JSON — backend will error clearly */
        }
      }
      let encPw: string | undefined;
      if (isEnc) {
        encPw = prompt(t("auth.importEncPwPrompt")) ?? undefined;
        if (!encPw) {
          setImportBusy(false);
          return;
        }
      }
      try {
        await importMafile(importFor, src, encPw);
      } catch (err: any) {
        const msg = String(err);
        if (msg.includes("MAFILE_NEEDS_PASSWORD") && !encPw) {
          const pw = prompt(t("auth.importEncPwPrompt"));
          if (!pw) throw err;
          await importMafile(importFor, src, pw);
        } else {
          throw err;
        }
      }
      toast("success", t("auth.import.success", { login: importFor }));
      setImportFor(null);
      setImportPath("");
      setImportJson("");
      setSelected(importFor);
    } catch (e: any) {
      toast("error", String(e));
    } finally {
      setImportBusy(false);
    }
  };

  const doExport = async () => {
    if (!active) return;
    setBusy(true);
    try {
      const target = await pickSaveFile(
        t("auth.export"),
        `${active.login}.maFile`,
        ["maFile", "json"],
      );
      if (!target) return;
      await exportMafile(active.login, target);
      toast("success", t("auth.export.success"));
    } catch (e: any) {
      toast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const doRemove = () => {
    if (!active) return;
    setRemoveAuthFor(active.login);
  };

  const confirmRemoveAuth = async () => {
    if (!removeAuthFor) return;
    setRemoveAuthBusy(true);
    try {
      await removeAuthenticator(removeAuthFor);
      toast("success", t("auth.remove"));
      setRemoveAuthFor(null);
    } catch (e: any) {
      toast("error", String(e));
    } finally {
      setRemoveAuthBusy(false);
    }
  };

  const doSyncTime = async () => {
    try {
      await api.authSyncTime();
      toast("success", t("auth.syncTimeDone"));
      if (selected) await refreshCode(selected);
    } catch (e: any) {
      toast("error", String(e));
    }
  };

  return (
    <div className="auth-view">
      {locked && (
        <div className="auth-lock-overlay">
          <div className="auth-lock-card">
            <div className="auth-lock-title">🔒 {t("auth.locked")}</div>
            <div className="auth-lock-hint">{t("auth.lockedHint")}</div>
            <input
              type="password"
              value={unlockPw}
              onChange={(e) => setUnlockPw(e.target.value)}
              placeholder={t("auth.security.unlockPwPh")}
              onKeyDown={async (e) => {
                if (e.key === "Enter" && unlockPw && !unlockBusy) {
                  setUnlockBusy(true);
                  try {
                    await unlockAuth(unlockPw);
                    setUnlockPw("");
                  } finally {
                    setUnlockBusy(false);
                  }
                }
              }}
              autoFocus
            />
            <button
              className="primary"
              disabled={unlockBusy || !unlockPw}
              onClick={async () => {
                setUnlockBusy(true);
                try {
                  await unlockAuth(unlockPw);
                  setUnlockPw("");
                } finally {
                  setUnlockBusy(false);
                }
              }}
            >
              {unlockBusy ? <Spinner size="xs" inline /> : t("auth.security.unlock")}
            </button>
          </div>
        </div>
      )}
      {!locked && <><div className="auth-head">
        <div>
          <h1 className="auth-title">{t("auth.title")}</h1>
          <div className="auth-subtitle">{t("auth.subtitle")}</div>
        </div>
        <div className="auth-head-actions">
          <button
            className="xs primary"
            onClick={() => setAddAccountOpen(true)}
            title={t("auth.addAccountHint")}
          >
            + {t("auth.addAccount")}
          </button>
          <button className="xs" onClick={doSyncTime}>
            ⟳ {t("auth.syncTime")}
          </button>
          <button className="xs" onClick={() => refreshAuthStatus()}>
            {t("auth.refresh")}
          </button>
        </div>
      </div>

      {allRail.length === 0 ? (
        <div className="auth-empty">
          <div className="auth-empty-title">{t("auth.emptyTitle")}</div>
          <div className="auth-empty-hint">{t("auth.emptyHint")}</div>
          <div className="auth-empty-actions">
            <button className="primary" onClick={() => setAddAccountOpen(true)}>
              + {t("auth.addAccount")}
            </button>
          </div>
        </div>
      ) : (
        <div className="auth-body">
          <div className="auth-rail">
            {allRail.map(({ a, kind }) => (
              <button
                key={a.login}
                className={
                  `auth-rail-item${selected === a.login ? " active" : ""}` +
                  (kind === "without" ? " dim" : "")
                }
                onClick={() => setSelected(a.login)}
                title={kind === "without" ? t("auth.noAuthYet") : undefined}
              >
                <div className="auth-rail-avatar">
                  {a.avatarPath ? (
                    <img src={convertFileSrc(a.avatarPath)} alt="" draggable={false} />
                  ) : (
                    <span className="avatar-fallback">
                      {(a.displayName || a.login).charAt(0).toUpperCase()}
                    </span>
                  )}
                </div>
                <div className="auth-rail-meta">
                  <div className="auth-rail-name">{a.displayName || a.login}</div>
                  <div className="auth-rail-login">
                    {kind === "without" ? t("auth.noAuthShort") : `@${a.login}`}
                  </div>
                </div>
              </button>
            ))}
          </div>
          <div className="auth-panel">
            {active && authStatus[active.login]?.identityMismatch && <ErrorBox message={t("error.AUTH_ACCOUNT_MISMATCH")} />}
            {active && authStatus[active.login]?.enrollment && authStatus[active.login].enrollment !== "none" && (
              <div className="recovery-banner">
                <span>{t("auth.add.resumeHint")}</span>
                <button className="primary" onClick={() => setAddFor(active.login)}>{t("auth.add.resume")}</button>
              </div>
            )}
            {active && withAuth.find((a) => a.login === active.login) && (
              <div className="auth-stack">
                <div className={`auth-codebar${remaining <= 5 ? " pulse" : ""}`}>
                  <div className="auth-codebar-code">
                    {code ? code.code : "·····"}
                  </div>
                  <div className="auth-codebar-ring">
                    <div
                      className="auth-codebar-bar"
                      style={{ width: `${Math.max(0, Math.min(100, (remaining / 30) * 100))}%` }}
                    />
                    <div className="auth-codebar-remaining">{remaining}s</div>
                  </div>
                  <div className="auth-codebar-actions">
                    <button
                      className="xs primary"
                      disabled={!code}
                      title={t("auth.copy")}
                      onClick={async () => {
                        if (!code) return;
                        try {
                          await copyGuardCode(active.login);
                          toast("success", t("auth.copied"));
                        } catch (e: any) {
                          toast("error", String(e));
                        }
                      }}
                    >
                      <Icon name="copy" />
                    </button>
                    <button className="xs icon-button" disabled={busy} aria-label={t("design.more")} onClick={e => { const r = e.currentTarget.getBoundingClientRect(); setMenu({ x: r.left, y: r.bottom + 4 }); }}><Icon name="more" /></button>
                  </div>
                </div>
                <div className="auth-codebar-meta">
                  <span>{t("auth.accountName")}: {authStatus[active.login]?.accountName ?? active.login} · {authStatus[active.login]?.steamId ?? active.steamId ?? "?"}</span>
                  {active.authenticatorImportedAt && (
                    <span>
                      {" · "}
                      {t("auth.imported")}:{" "}
                      {new Date(
                        parseInt(active.authenticatorImportedAt) * 1000,
                      ).toLocaleDateString()}
                    </span>
                  )}
                </div>
                {(() => {
                  const st = sessionStates[active.login] ?? "ok";
                  if (st === "ok") return null;
                  const auto = authStatus[active.login]?.autoLogin?.state;
                  const refreshable = st === "refreshable" || auto === "ready";
                  return (
                    <div className={`auth-session-badge ${st}`}>
                      <span className="auth-session-badge-icon">
                        {refreshable ? "⟳" : "⚠"}
                      </span>
                      <div className="auth-session-badge-text">
                        <div className="auth-session-badge-title">
                          {refreshable
                            ? t("auth.session.recoverTitle")
                            : t("auth.session.expiredTitle")}
                        </div>
                        <div className="auth-session-badge-hint">
                          {refreshable
                            ? t("auth.session.recoverHint")
                            : t("auth.session.expiredHint")}
                        </div>
                      </div>
                      <button
                        className="xs primary"
                        disabled={refreshBusy}
                        onClick={async () => {
                          if (refreshable) {
                            setRefreshBusy(true);
                            try {
                              const ok = await refreshAccessToken(active.login);
                              if (ok) {
                                toast("success", t("auth.session.refreshed"));
                                await refreshConfirmations(active.login);
                              }
                            } finally {
                              setRefreshBusy(false);
                            }
                          } else {
                            setLoginFor(active.login);
                          }
                        }}
                      >
                        {refreshBusy ? (
                          <Spinner size="xs" inline />
                        ) : refreshable ? (
                          t("auth.session.refreshAction")
                        ) : (
                          t("auth.session.reloginAction")
                        )}
                      </button>
                    </div>
                  );
                })()}
                <ConfirmationsSection key={active.login} login={active.login} />
                {authStatus[active.login]?.hasSavedPassword && (
                  <div className="auth-codebar-meta">
                    <span>{t(`auth.auto.${authStatus[active.login]?.autoLogin?.state ?? "unavailable"}`)}
                    {authStatus[active.login]?.autoLogin?.retry_at ? ` ${new Date(authStatus[active.login].autoLogin.retry_at! * 1000).toLocaleTimeString()}` : ""}</span>
                    <button className="xs ghost" onClick={async () => {
                      try { await api.authPasswordForget(active.login); await refreshAuthStatus(); }
                      catch (e) { toast("error", String(e)); }
                    }}>{t("auth.login.forgetPassword")}</button>
                  </div>
                )}
              </div>
            )}
            {active && !withAuth.find((a) => a.login === active.login) && (
              <div className="auth-noauth">
                <div className="auth-noauth-title">{t("auth.noAuthYet")}</div>
                <div className="auth-noauth-hint">{t("auth.noAuthHint")}</div>
                <div className="auth-noauth-actions">
                  <button
                    className="primary"
                    onClick={() => setAddFor(active.login)}
                  >
                    + {t("auth.add.title")}
                  </button>
                  <button
                    className="xs"
                    onClick={() => setImportFor(active.login)}
                  >
                    ↓ {t("auth.import")}
                  </button>
                  <button
                    className="xs"
                    onClick={async () => {
                      try {
                        await api.authOpenFolder(active.login);
                      } catch (e: any) {
                        toast("error", String(e));
                      }
                    }}
                  >
                    ⛶ {t("auth.openFolder")}
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {importFor && (
        <div className="modal-backdrop" onClick={() => !importBusy && setImportFor(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">
              {t("auth.import")} → {importFor}
            </div>
            <div className="modal-body">
              <div className="field">
                <label>{t("auth.pickFileShort")}</label>
                <div style={{ display: "flex", gap: 8 }}>
                  <input
                    value={importPath}
                    onChange={(e) => setImportPath(e.target.value)}
                    placeholder="C:\\...\\12345.maFile"
                  />
                  <button
                    className="xs"
                    onClick={async () => {
                      const p = await pickFile(t("auth.import"), ["maFile", "json"]);
                      if (p) setImportPath(p);
                    }}
                  >
                    …
                  </button>
                </div>
              </div>
              <div className="field">
                <label>{t("auth.inlineJson")}</label>
                <textarea
                  value={importJson}
                  onChange={(e) => setImportJson(e.target.value)}
                  rows={5}
                  spellCheck={false}
                  placeholder='{ "shared_secret": "...", "identity_secret": "...", "account_name": "..." }'
                />
              </div>
              <div className="hint">{t("auth.importHint")}</div>
            </div>
            <div className="modal-actions">
              <button
                className="xs"
                disabled={importBusy}
                onClick={() => setImportFor(null)}
              >
                {t("common.cancel")}
              </button>
              <button
                className="primary"
                disabled={importBusy || (!importPath.trim() && !importJson.trim())}
                onClick={doImport}
              >
                {importBusy ? <Spinner size="xs" inline /> : t("auth.import.submit")}
              </button>
            </div>
          </div>
        </div>
      )}
      {menu && active && <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} items={[
        { label: t("auth.login.openInCard"), onClick: () => setLoginFor(active.login) },
        { label: t("auth.openFolderHint"), onClick: () => { void api.authOpenFolder(active.login).catch(e => toast("error", String(e))); } },
        { label: t("auth.export"), onClick: () => { void doExport(); } },
        { divider: true },
        { label: t("auth.remove"), danger: true, onClick: doRemove },
      ]} />}
      {loginFor && (
        <LoginFlowModal
          open={true}
          login={loginFor}
          defaultAccountName={
            accounts.find((a) => a.login === loginFor)?.login ?? loginFor
          }
          onClose={() => setLoginFor(null)}
          onSuccess={() => void refreshConfirmations(loginFor)}
        />
      )}
      {addFor && (
        <AddAuthenticatorModal
          open={true}
          login={addFor}
          onClose={() => { setAddFor(null); void refreshAuthStatus(); }}
        />
      )}
      <AddAccountModal
        open={addAccountOpen}
        onClose={() => setAddAccountOpen(false)}
        onSubmit={async (login, display) => {
          await add(login, display);
          setSelected(login);
          // Open the AddAuthenticator wizard immediately so the user gets to
          // the "attach SDA" flow without an extra click.
          setTimeout(() => setAddFor(login), 200);
        }}
      />
      <ConfirmDialog
        open={!!removeAuthFor}
        title={t("auth.removeConfirmTitle", { login: removeAuthFor ?? "" })}
        body={t("auth.removeConfirmBody")}
        bullets={[
          t("auth.removeConfirmBullet1"),
          t("auth.removeConfirmBullet2"),
          t("auth.removeConfirmBullet3"),
        ]}
        requireText={removeAuthFor ?? ""}
        requireHint={t("confirm.typeLoginToConfirm", { login: removeAuthFor ?? "" })}
        confirmLabel={t("auth.removeConfirmButton")}
        busy={removeAuthBusy}
        onCancel={() => !removeAuthBusy && setRemoveAuthFor(null)}
        onConfirm={confirmRemoveAuth}
      />
      </>}
    </div>
  );
}
