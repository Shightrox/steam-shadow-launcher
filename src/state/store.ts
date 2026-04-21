import { create } from "zustand";
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
  codes: Record<string, GuardCode>;
  confirmations: Record<string, Confirmation[]>;
  confLoading: Record<string, boolean>;
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
  importMafile(login: string, source: string, encryptionPassword?: string): Promise<void>;
  exportMafile(login: string, target: string): Promise<void>;
  removeAuthenticator(login: string): Promise<void>;
  refreshCode(login: string): Promise<void>;
  refreshConfirmations(login: string): Promise<void>;
  mergeConfirmations(login: string, items: Confirmation[]): void;
  respondConfirmations(login: string, ids: string[], op: ConfirmOp): Promise<boolean>;
  refreshAuthLock(): Promise<void>;
  unlockAuth(password: string): Promise<boolean>;
  lockAuth(): Promise<void>;
  setMasterPassword(oldPw: string | null, newPw: string | null): Promise<boolean>;
  setAddAuthActive(v: boolean): void;
}

export const useApp = create<AppState>((set, get) => ({
  settings: null,
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
  codes: {},
  confirmations: {},
  confLoading: {},
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
    try {
      const settings = await api.getSettings();
      set({ settings });
      try {
        const mainSteam = await api.detectMainSteam();
        set({ mainSteam, mainSteamError: null });
      } catch (e: any) {
        set({ mainSteam: null, mainSteamError: String(e) });
      }
      await get().refreshSandboxie();
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
        await get().refreshAccounts();
      }
      // Authenticator status is cheap and useful even before any maFile is
      // imported (drives the per-card widget visibility).
      await get().refreshAuthStatus();
      await get().refreshAuthLock();
      // Push current poller config to the running thread (idempotent).
      try {
        await api.authPollerConfigure({
          enabled: settings.authPollerEnabled,
          interval: settings.authPollerInterval,
          autoConfirmTrades: settings.authAutoConfirmTrades,
          autoConfirmMarket: settings.authAutoConfirmMarket,
        });
      } catch (e: any) {
        get().log("warn", `authPollerConfigure: ${e}`);
      }
    } catch (e: any) {
      get().log("error", String(e));
    }
  },

  async refreshAccounts() {
    try {
      const accounts = await api.listAccounts();
      set({ accounts });
      await Promise.all(accounts.map((a) => get().refreshHealth(a.login)));
    } catch (e: any) {
      get().log("error", `listAccounts: ${e}`);
    }
  },

  async refreshHealth(login: string) {
    try {
      const h = await api.verifyAccount(login);
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
    try {
      const list = await api.authStatus();
      const map: Record<string, AccountAuthStatus> = {};
      for (const it of list) map[it.login] = it;
      set({ authStatus: map });
    } catch (e: any) {
      get().log("warn", `authStatus: ${e}`);
    }
  },

  async importMafile(login: string, source: string, encryptionPassword?: string) {
    await api.authImportMafile(login, source, encryptionPassword);
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
    await api.authRemove(login);
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
    try {
      const code = await api.authGenerateCode(login);
      set((s) => ({ codes: { ...s.codes, [login]: code } }));
    } catch (e: any) {
      // Quiet failure: a missing maFile or revoked secret shouldn't spam logs.
      console.warn(`refreshCode(${login}):`, e);
    }
  },

  async refreshConfirmations(login: string) {
    set((s) => ({ confLoading: { ...s.confLoading, [login]: true } }));
    try {
      const list = await api.authConfirmationsList(login);
      set((s) => ({ confirmations: { ...s.confirmations, [login]: list } }));
    } catch (e: any) {
      const msg = String(e);
      // Distinguish "needs relogin" from generic failure.
      if (msg.includes("CONF_NEEDS_RELOGIN") || msg.includes("CONF_NO_SESSION")) {
        get().toast("error", "Session expired — re-login required");
      } else {
        get().toast("error", `Confirmations: ${msg}`);
      }
      set((s) => ({ confirmations: { ...s.confirmations, [login]: [] } }));
    } finally {
      set((s) => ({ confLoading: { ...s.confLoading, [login]: false } }));
    }
  },

  mergeConfirmations(login: string, items: Confirmation[]) {
    set((s) => ({ confirmations: { ...s.confirmations, [login]: items } }));
  },

  async respondConfirmations(login: string, ids: string[], op: ConfirmOp) {
    try {
      const results = await api.authConfirmationsRespond(login, ids, op);
      const bad = results.filter((r) => !r.success);
      if (bad.length === 0) {
        get().toast("success", `${op === "allow" ? "Allowed" : "Rejected"}: ${ids.length}`);
      } else {
        get().toast(
          "error",
          `Partial failure: ${bad.length}/${results.length} (${bad[0]?.message ?? ""})`,
        );
      }
      await get().refreshConfirmations(login);
      return bad.length === 0;
    } catch (e: any) {
      get().toast("error", String(e));
      return false;
    }
  },

  async refreshAuthLock() {
    try {
      const lock = await api.authLockStatus();
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
    try {
      await api.authLock();
      set({ codes: {} });
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
