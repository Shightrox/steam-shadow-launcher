import { create } from "zustand";
import {
  api,
  type Account,
  type AccountHealth,
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
}));
