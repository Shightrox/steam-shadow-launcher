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

export function AuthenticatorView() {
  const { t } = useI18n();
  const {
    accounts,
    authStatus,
    authLock,
    codes,
    importMafile,
    exportMafile,
    removeAuthenticator,
    refreshCode,
    refreshAuthStatus,
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

  const [selected, setSelected] = useState<string | null>(null);
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

  const locked = !!authLock?.enabled && !authLock.unlocked && !!authLock.hasEncryptedFiles;

  // Keep a sane selection — pick from the unified list so accounts without
  // an authenticator can also be selected (so the user can attach one).
  useEffect(() => {
    if (!selected && allRail.length) setSelected(allRail[0].a.login);
    if (selected && !allRail.find(({ a }) => a.login === selected) && allRail.length) {
      setSelected(allRail[0].a.login);
    }
    if (!allRail.length && selected) setSelected(null);
  }, [allRail, selected]);

  // Pump the countdown every second locally (server-time is cached in Rust).
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const iv = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(iv);
  }, []);

  // If the code in store has run down, fetch a fresh one.
  useEffect(() => {
    if (!selected) return;
    const c = codes[selected];
    if (!c) {
      refreshCode(selected);
    } else if (c.periodRemaining <= 1) {
      const tm = setTimeout(() => refreshCode(selected), 500);
      return () => clearTimeout(tm);
    }
    return undefined;
  }, [selected, codes, tick]);

  const active = selected
    ? accounts.find((a) => a.login === selected) ?? null
    : null;
  const code = active ? codes[active.login] : undefined;
  const remaining = code
    ? Math.max(0, code.periodRemaining - Math.floor((Date.now() / 1000) - code.generatedAt))
    : 0;

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
                if (e.key === "Enter" && unlockPw) {
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
      <div className="auth-head">
        <div>
          <div className="auth-title">{t("auth.title")}</div>
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
                          await navigator.clipboard.writeText(code.code);
                          toast("success", t("auth.copied"));
                        } catch (e: any) {
                          toast("error", String(e));
                        }
                      }}
                    >
                      ⎘
                    </button>
                    <button
                      className="xs"
                      disabled={busy}
                      onClick={async () => {
                        try {
                          await api.authOpenFolder(active.login);
                        } catch (e: any) {
                          toast("error", String(e));
                        }
                      }}
                      title={t("auth.openFolderHint")}
                    >
                      ⛶
                    </button>
                    <button
                      className="xs"
                      disabled={busy}
                      onClick={() => setLoginFor(active.login)}
                      title={t("auth.login.openInCard")}
                    >
                      ⎙
                    </button>
                    <button
                      className="xs"
                      disabled={busy}
                      onClick={doExport}
                      title={t("auth.export")}
                    >
                      ↑
                    </button>
                    <button
                      className="xs danger ghost"
                      disabled={busy}
                      onClick={doRemove}
                      title={t("auth.remove")}
                    >
                      ×
                    </button>
                  </div>
                </div>
                <div className="auth-codebar-meta">
                  <span>{t("auth.accountName")}: {active.login}</span>
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
                <ConfirmationsSection login={active.login} />
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
      {loginFor && (
        <LoginFlowModal
          open={true}
          login={loginFor}
          defaultAccountName={
            accounts.find((a) => a.login === loginFor)?.login ?? loginFor
          }
          onClose={() => setLoginFor(null)}
        />
      )}
      {addFor && (
        <AddAuthenticatorModal
          open={true}
          login={addFor}
          onClose={() => setAddFor(null)}
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
    </div>
  );
}

function typeLabel(t: (k: string) => string, kind: number): string {
  switch (kind) {
    case 1:
    case 2:
      return t("auth.typeTrade");
    case 3:
      return t("auth.typeMarket");
    case 6:
      return t("auth.typePhone");
    case 8:
      return t("auth.typeAccountRecovery");
    case 9:
      return t("auth.typeLogin");
    default:
      return t("auth.typeOther");
  }
}

function typeIcon(kind: number): string {
  switch (kind) {
    case 1:
    case 2:
      return "⇄";
    case 3:
      return "$";
    case 6:
      return "☎";
    case 8:
      return "⚷";
    case 9:
      return "⌨";
    default:
      return "?";
  }
}

function ConfirmationsSection({ login }: { login: string }) {
  const { t } = useI18n();
  const confirmations = useApp((s) => s.confirmations[login]);
  const loading = useApp((s) => !!s.confLoading[login]);
  const refreshConfirmations = useApp((s) => s.refreshConfirmations);
  const respondConfirmations = useApp((s) => s.respondConfirmations);
  const [selection, setSelection] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    refreshConfirmations(login);
    setSelection({});
  }, [login]);

  const list = confirmations ?? [];
  const selectedIds = Object.entries(selection)
    .filter(([, v]) => v)
    .map(([id]) => id);
  const anySelected = selectedIds.length > 0;

  const doRespond = async (op: "allow" | "reject") => {
    const ids = anySelected ? selectedIds : list.map((c) => c.id);
    if (!ids.length) return;
    setBusy(true);
    try {
      await respondConfirmations(login, ids, op);
      setSelection({});
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="auth-conf">
      <div className="auth-conf-head">
        <div className="auth-conf-title">
          {t("auth.confirmations")}
          {list.length > 0 && (
            <span className="auth-conf-count">{list.length}</span>
          )}
        </div>
        <button
          className="xs"
          disabled={loading}
          onClick={() => refreshConfirmations(login)}
        >
          {loading ? <Spinner size="xs" inline /> : "⟳"}
        </button>
      </div>
      {list.length === 0 ? (
        <div className="auth-conf-empty">
          {loading ? <Spinner size="sm" /> : t("auth.confirmations.empty")}
        </div>
      ) : (
        <>
          <div className="auth-conf-list">
            {list.map((c) => {
              const checked = !!selection[c.id];
              return (
                <label
                  key={c.id}
                  className={`auth-conf-row${checked ? " checked" : ""}`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) =>
                      setSelection((sel) => ({ ...sel, [c.id]: e.target.checked }))
                    }
                  />
                  <span className="auth-conf-icon">{typeIcon(c.type)}</span>
                  <div className="auth-conf-meta">
                    <div className="auth-conf-headline">{c.headline}</div>
                    {c.summary?.length > 0 && (
                      <div className="auth-conf-summary">
                        {c.summary.join(" · ")}
                      </div>
                    )}
                    <div className="auth-conf-type">
                      {typeLabel(t, c.type)}
                    </div>
                  </div>
                </label>
              );
            })}
          </div>
          <div className="auth-conf-actions">
            <button
              className="primary"
              disabled={busy}
              onClick={() => doRespond("allow")}
              title={anySelected ? t("auth.allowSelected") : t("auth.allowAll")}
            >
              ✓ {anySelected ? t("auth.allowSelected") : t("auth.allowAll")}
            </button>
            <button
              className="xs danger ghost"
              disabled={busy}
              onClick={() => doRespond("reject")}
              title={anySelected ? t("auth.rejectSelected") : t("auth.rejectAll")}
            >
              ✕ {anySelected ? t("auth.rejectSelected") : t("auth.rejectAll")}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
