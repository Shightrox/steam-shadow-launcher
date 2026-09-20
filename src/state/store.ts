import { setCurrentWorkspace } from "./workspaceContext";
import { create } from "zustand";
import { useI18n } from "../i18n";
import {
  api,
  type Account,
  type AccountAuthStatus,
  type AccountHealth,
  type AuthLockStatus,
  type Confirmation,
  type ConfirmOp,
  type GuardCode,
  type LaunchMode,
  type MainSteamInfo,
  type SandboxieInfo,
  type SessionState,
  type Settings,
} from "../api/tauri";

interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  msg: string;
}

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  msg: string;
}

export type SbStatus = "unknown" | "ready" | "missing" | "installing" | "failed";

interface AppState {
  settings: Settings | null;
  bootError: string | null;
  workspaceEpoch: number;
  mainSteam: MainSteamInfo | null;
  mainSteamError: string | null;
  accounts: Account[];
  healths: Record<string, AccountHealth>;
  logs: LogEntry[];
  pendingImport: boolean;
  sandboxie: SandboxieInfo | null;
  sbStatus: SbStatus;
  toasts: Toast[];
  launchingLogin: string | null;
  authStatus: Record<string, AccountAuthStatus>;
  sessionStates: Record<string, SessionState>;
  codes: Record<string, GuardCode>;
  confirmations: Record<string, Confirmation[]>;
  confLoading: Record<string, boolean>;
  confErrors: Record<string, string | null>;
  authLock: AuthLockStatus | null;
  /// True while the AddAuthenticator wizard is mounted. Used by TitleBar to
  /// prompt-to-confirm before closing the window mid-flow — at certain phases
  /// (post-finalize, pre-persist) closing would strand the user with an
  /// activated-but-unsaved authenticator.
  addAuthActive: boolean;
  toast(kind: Toast["kind"], msg: string): void;
  dismissToast(id: number): void;
  setLaunching(login: string | null): void;
  log(level: LogEntry["level"], msg: string): void;
  bootstrap(): Promise<void>;
  refreshAccounts(): Promise<void>;
  refreshHealth(login: string): Promise<void>;
  launch(login: string, mode?: LaunchMode): Promise<void>;
  add(login: string, display: string | null): Promise<void>;
  remove(login: string, deleteFiles: boolean): Promise<void>;
  repair(login: string): Promise<void>;
  setFavorite(login: string, value: boolean): Promise<void>;
  refreshAvatar(login: string): Promise<void>;
  triggerImportPrompt(): void;
  clearImportPrompt(): void;
  setDefaultMode(m: LaunchMode): Promise<void>;
  refreshSandboxie(): Promise<void>;
  installSandboxie(installerPath: string): Promise<void>;
  downloadAndInstallSandboxie(): Promise<boolean>;
  refreshAuthStatus(): Promise<void>;
  refreshSessionState(login: string): Promise<void>;
  setSessionState(login: string, state: SessionState): void;
  refreshAccessToken(login: string): Promise<boolean>;
  importMafile(login: string, source: string, encryptionPassword?: string): Promise<void>;
  exportMafile(login: string, target: string): Promise<void>;
  removeAuthenticator(login: string): Promise<void>;
  refreshCode(login: string): Promise<GuardCode | undefined>;
  refreshConfirmations(login: string): Promise<void>;
  mergeConfirmations(login: string, items: Confirmation[]): void;
  setConfirmationError(login: string, message: string): void;
  respondConfirmations(login: string, ids: string[], op: ConfirmOp): Promise<boolean>;
  refreshAuthLock(): Promise<void>;
  unlockAuth(password: string): Promise<boolean>;
  lockAuth(): Promise<void>;
  setMasterPassword(oldPw: string | null, newPw: string | null): Promise<boolean>;
  setAddAuthActive(v: boolean): void;
}

const codeRequests = new Map<string, Promise<GuardCode | undefined>>();
const codeRetryAt = new Map<string, number>();

export const useApp = create<AppState>((set, get) => ({
  settings: null,
  bootError: null,
  workspaceEpoch: 0,
  mainSteam: null,
  mainSteamError: null,
  accounts: [],
  healths: {},
  logs: [],
  pendingImport: false,
  sandboxie: null,
  sbStatus: "unknown",
  toasts: [],
  launchingLogin: null,
  authStatus: {},
  sessionStates: {},
  codes: {},
  confirmations: {},
  confLoading: {},
  confErrors: {},
  authLock: null,
  addAuthActive: false,

  toast(kind, msg) {
    const id = Date.now() + Math.random();
    set((s) => ({ toasts: [...s.toasts, { id, kind, msg }] }));
    setTimeout(() => {
      set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) }));
    }, kind === "error" ? 6000 : 4000);
  },
  dismissToast(id) {
    set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) }));
  },
  setLaunching(login) {
    set({ launchingLogin: login });
  },

  log(level, msg) {
    const entry: LogEntry = { ts: Date.now(), level, msg };
    set((s) => ({ logs: [entry, ...s.logs].slice(0, 200) }));
    if (level === "error") console.error(msg);
    else console.log(msg);
  },

  triggerImportPrompt() {
    set({ pendingImport: true });
  },
  clearImportPrompt() {
    set({ pendingImport: false });
  },

  async bootstrap() {
    const epoch = get().workspaceEpoch + 1;
    set({ workspaceEpoch: epoch, bootError: null, settings: null, accounts: [], healths: {},
      codes: {}, confirmations: {}, confErrors: {}, confLoading: {}, sessionStates: {}, authStatus: {}, authLock: null });
    setCurrentWorkspace(null);
    codeRequests.clear(); codeRetryAt.clear();
    try {
      const settings = await api.getSettings();
      if (epoch !== get().workspaceEpoch) return;
      setCurrentWorkspace(settings.workspace);
      set({ settings });
      try {
        const mainSteam = await api.detectMainSteam();
        if (epoch !== get().workspaceEpoch) return;
        set({ mainSteam, mainSteamError: null });
      } catch (e: any) {
        if (epoch !== get().workspaceEpoch) return;
        set({ mainSteam: null, mainSteamError: String(e) });
      }
      await get().refreshSandboxie();
      if (epoch !== get().workspaceEpoch) return;
      if (settings.firstRunCompleted && settings.workspace) {
        try {
          const report = await api.cleanupStaleJunctions();
          if (report.repaired.length || report.errors.length) {
            get().log(
              "info",
              `Junction sweep: repaired=${report.repaired.length}, errors=${report.errors.length}`
            );
          }
        } catch (e: any) {
          get().log("warn", `cleanup_stale_junctions: ${e}`);
        }
        if (epoch !== get().workspaceEpoch) return;
        await get().refreshAccounts();
      }
      if (epoch !== get().workspaceEpoch) return;
      // Authenticator status is cheap and useful even before any maFile is
      // imported (drives the per-card widget visibility).
      await get().refreshAuthStatus();
      await get().refreshAuthLock();
    } catch (e: any) {
      if (epoch !== get().workspaceEpoch) return;
      set({ bootError: String(e) });
      get().log("error", String(e));
    }
  },

  async refreshAccounts() {
    const epoch = get().workspaceEpoch;
    try {
      const accounts = await api.listAccounts();
      if (epoch !== get().workspaceEpoch) return;
      set({ accounts });
      await Promise.all(accounts.map((a) => get().refreshHealth(a.login)));
    } catch (e: any) {
      get().log("error", `listAccounts: ${e}`);
    }
  },

  async refreshHealth(login: string) {
    const epoch = get().workspaceEpoch;
    try {
      const h = await api.verifyAccount(login);
      if (epoch !== get().workspaceEpoch) return;
      set((s) => ({ healths: { ...s.healths, [login]: h } }));
    } catch (e: any) {
      get().log("warn", `verifyAccount ${login}: ${e}`);
    }
  },

  async launch(login: string, mode?: LaunchMode) {
    const out = await api.launchShadow(login, mode);
    get().log(
      "info",
      `Launched (${out.kind}): ${login} pid=${out.pid}`
    );
  },

  async add(login: string, display: string | null) {
    await api.addAccount(login, display);
    get().log("info", `Account added: ${login}`);
    await get().refreshAccounts();
  },

  async remove(login: string, deleteFiles: boolean) {
    await api.removeAccount(login, deleteFiles);
    get().log("info", `Account removed: ${login}`);
    await get().refreshAccounts();
  },

  async repair(login: string) {
    await api.repairAccount(login);
    get().log("info", `Repaired: ${login}`);
    await get().refreshHealth(login);
  },

  async setFavorite(login: string, value: boolean) {
    await api.setAccountFavorite(login, value);
    await get().refreshAccounts();
  },

  async refreshAvatar(login: string) {
    try {
      await api.refreshAccountAvatar(login);
      await get().refreshAccounts();
    } catch (e) {
      get().log("warn", `Avatar refresh failed for ${login}: ${e}`);
    }
  },

  async setDefaultMode(m: LaunchMode) {
    const s = get().settings;
    if (!s) return;
    const next = { ...s, defaultLaunchMode: m };
    await api.saveSettings(next);
    set({ settings: next });
    get().log("info", `Default mode: ${m}`);
  },

  async refreshSandboxie() {
    try {
      const sb = await api.detectSandboxie();
      set({
        sandboxie: sb,
        sbStatus: sb.installed ? "ready" : "missing",
      });
    } catch (e: any) {
      set({ sbStatus: "failed" });
      get().log("warn", `detectSandboxie: ${e}`);
    }
  },

  async installSandboxie(installerPath: string) {
    set({ sbStatus: "installing" });
    try {
      const sb = await api.installSandboxie(installerPath);
      set({
        sandboxie: sb,
        sbStatus: sb.installed ? "ready" : "failed",
      });
      get().log("info", "Sandboxie installed");
    } catch (e: any) {
      set({ sbStatus: "failed" });
      get().log("error", `installSandboxie: ${e}`);
    }
  },

  async downloadAndInstallSandboxie() {
    set({ sbStatus: "installing" });
    try {
      const sb = await api.downloadAndInstallSandboxie();
      set({
        sandboxie: sb,
        sbStatus: sb.installed ? "ready" : "failed",
      });
      get().log("info", "Sandboxie auto-installed");
      return sb.installed;
    } catch (e: any) {
      set({ sbStatus: "failed" });
      get().log("error", `downloadAndInstallSandboxie: ${e}`);
      return false;
    }
  },

  async refreshAuthStatus() {
    const epoch = get().workspaceEpoch;
    try {
      const list = await api.authStatus();
      if (epoch !== get().workspaceEpoch) return;
      const map: Record<string, AccountAuthStatus> = {};
      const states: Record<string, SessionState> = {};
      for (const it of list) {
        map[it.login] = it;
        states[it.login] = it.sessionState;
      }
      set({ authStatus: map, sessionStates: states });
    } catch (e: any) {
      get().log("warn", `authStatus: ${e}`);
    }
  },

  async refreshSessionState(login: string) {
    const epoch = get().workspaceEpoch;
    try {
      const st = await api.authSessionState(login);
      if (epoch !== get().workspaceEpoch) return;
      set((s) => ({ sessionStates: { ...s.sessionStates, [login]: st } }));
    } catch (e: any) {
      console.warn(`refreshSessionState(${login}):`, e);
    }
  },

  setSessionState(login, state) {
    set((s) => ({ sessionStates: { ...s.sessionStates, [login]: state } }));
  },

  async refreshAccessToken(login: string) {
    const epoch = get().workspaceEpoch;
    try {
      await api.authLoginRefresh(login);
      if (epoch !== get().workspaceEpoch) return false;
      await get().refreshAuthStatus();
      return true;
    } catch (e: any) {
      if (epoch !== get().workspaceEpoch) return false;
      get().toast("error", String(e));
      await get().refreshAuthStatus();
      return false;
    }
  },

  async importMafile(login: string, source: string, encryptionPassword?: string) {
    const epoch = get().workspaceEpoch;
    await api.authImportMafile(login, source, encryptionPassword);
      if (epoch !== get().workspaceEpoch) return ;
    get().log("info", `Authenticator imported: ${login}`);
    await get().refreshAccounts();
    await get().refreshAuthStatus();
    await get().refreshCode(login);
  },

  async exportMafile(login: string, target: string) {
    await api.authExportMafile(login, target);
    get().log("info", `Authenticator exported: ${login} -> ${target}`);
  },

  async removeAuthenticator(login: string) {
    const epoch = get().workspaceEpoch;
    await api.authRemove(login);
      if (epoch !== get().workspaceEpoch) return ;
    get().log("info", `Authenticator removed: ${login}`);
    set((s) => {
      const codes = { ...s.codes };
      delete codes[login];
      const authStatus = { ...s.authStatus };
      delete authStatus[login];
      return { codes, authStatus };
    });
    await get().refreshAccounts();
  },

  async refreshCode(login: string) {
    const epoch = get().workspaceEpoch;
    if (get().authLock?.enabled && !get().authLock?.unlocked) return;
    const key = `${epoch}:${login}`;
    const running = codeRequests.get(key);
    if (running) return running;
    if ((codeRetryAt.get(key) ?? 0) > Date.now()) return;
    const job = (async () => {
      try {
        const code = await api.authGenerateCode(login);
        if (epoch !== get().workspaceEpoch || (get().authLock?.enabled && !get().authLock?.unlocked)) return;
        set((s) => ({ codes: { ...s.codes, [login]: code } }));
        return code;
      } catch {
        codeRetryAt.set(key, Date.now() + 5000);
        if (epoch === get().workspaceEpoch) set((s) => { const codes = { ...s.codes }; delete codes[login]; return { codes }; });
      } finally { codeRequests.delete(key); }
    })();
    codeRequests.set(key, job);
    return job;
  },

  async refreshConfirmations(login: string) {
    const epoch = get().workspaceEpoch;
    if (get().confLoading[login]) return;
    set((s) => ({ confLoading: { ...s.confLoading, [login]: true } }));
    try {
      const list = await api.authConfirmationsList(login);
      if (epoch !== get().workspaceEpoch) return ;
      get().mergeConfirmations(login, list);
      get().setSessionState(login, "ok");
    } catch (e: any) {
      if (epoch !== get().workspaceEpoch) return ;
      const msg = String(e);
      get().setConfirmationError(login, msg);
    } finally {
      if (epoch === get().workspaceEpoch) set((s) => ({ confLoading: { ...s.confLoading, [login]: false } }));
    }
  },

  mergeConfirmations(login: string, items: Confirmation[]) {
    if (get().authLock?.enabled && !get().authLock?.unlocked) return;
    set((s) => ({
      confirmations: { ...s.confirmations, [login]: items },
      confErrors: { ...s.confErrors, [login]: null },
    }));
  },

  setConfirmationError(login: string, message: string) {
    set((s) => ({ confErrors: { ...s.confErrors, [login]: message } }));
    if (message.includes("CONF_NEEDS_RELOGIN") || /AUTH_(AUTO_LOGIN|PASSWORD_UNAVAILABLE|LOGIN_REJECTED|GUARD_REJECTED|ACCOUNT_MISMATCH)/.test(message)) {
      get().setSessionState(login, "needs_relogin");
      void get().refreshAuthStatus();
    } else if (/CONF_NO_(SESSION|ACCESS_TOKEN|STEAM_ID)/.test(message)) {
      get().setSessionState(login, "no_session");
    }
  },

  async respondConfirmations(login: string, ids: string[], op: ConfirmOp) {
    const epoch = get().workspaceEpoch;
    try {
      const results = await api.authConfirmationsRespond(login, ids, op);
      if (epoch !== get().workspaceEpoch) return false;
      const bad = results.filter((r) => !r.success);
      if (bad.length === 0) {
        get().toast("success", useI18n.getState().t(op === "allow" ? "design.allowed" : "design.rejected", { count: ids.length }));
      } else {
        get().toast(
          "error",
          useI18n.getState().t("design.partialFailure", { count: bad.length, total: results.length, message: bad[0]?.message ?? "" }),
        );
      }
      await get().refreshConfirmations(login);
      if (bad.length) get().setConfirmationError(login, bad[0].message || "Steam rejected the confirmation request");
      return bad.length === 0;
    } catch (e: any) {
      if (epoch !== get().workspaceEpoch) return false;
      get().setConfirmationError(login, String(e));
      get().toast("error", String(e));
      return false;
    }
  },

  async refreshAuthLock() {
    const epoch = get().workspaceEpoch;
    try {
      const lock = await api.authLockStatus();
      if (epoch !== get().workspaceEpoch) return;
      set({ authLock: lock });
    } catch (e: any) {
      get().log("warn", `authLockStatus: ${e}`);
    }
  },

  async unlockAuth(password: string) {
    try {
      await api.authUnlock(password);
      await get().refreshAuthLock();
      get().toast("success", "Unlocked");
      // Refresh codes for any accounts whose secrets just became readable.
      for (const a of get().accounts) {
        if (a.hasAuthenticator) await get().refreshCode(a.login);
      }
      return true;
    } catch (e: any) {
      get().toast("error", String(e));
      return false;
    }
  },

  async lockAuth() {
    set(s => ({ workspaceEpoch: s.workspaceEpoch + 1, codes: {}, confirmations: {}, confErrors: {}, confLoading: {}, authLock: s.authLock ? { ...s.authLock, unlocked: false } : null }));
    codeRequests.clear(); codeRetryAt.clear();
    try {
      await api.authLock();
      set({ codes: {}, confirmations: {}, confErrors: {} });
      await get().refreshAuthLock();
    } catch (e: any) {
      get().toast("error", String(e));
    }
  },

  async setMasterPassword(oldPw: string | null, newPw: string | null) {
    try {
      await api.authSetMasterPassword(oldPw, newPw);
      // Sync settings so UI reflects new state.
      const settings = await api.getSettings();
      set({ settings });
      await get().refreshAuthLock();
      get().toast(
        "success",
        newPw ? "Master password set" : "Master password disabled",
      );
      return true;
    } catch (e: any) {
      get().toast("error", String(e));
      return false;
    }
  },
  setAddAuthActive(v) {
    set({ addAuthActive: v });
  },
}));
