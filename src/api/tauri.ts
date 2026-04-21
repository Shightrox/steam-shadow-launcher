import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface MainSteamInfo {
  installDir: string;
  steamExe: string;
  steamappsDir: string;
  autologinUser: string | null;
}

export type LaunchMode = "switch" | "sandbox";

export interface Settings {
  version: number;
  workspace: string | null;
  mainSteamPathOverride: string | null;
  firstRunCompleted: boolean;
  language: string;
  defaultLaunchMode: LaunchMode;
  sandboxieInstallAttempted: boolean;
}

export interface Account {
  login: string;
  displayName: string | null;
  path: string;
  lastLaunchAt: string | null;
  steamId: string | null;
  avatarPath: string | null;
  favorite: boolean;
  launchCount: number;
}

export type JunctionHealth =
  | { kind: "healthy" }
  | { kind: "missing" }
  | { kind: "stale"; actual: string }
  | { kind: "notajunction" };

export interface AccountHealth {
  junction: JunctionHealth;
  configDirExists: boolean;
  hasLoginusersVdf: boolean;
  ready: boolean;
}

export interface CleanupReport {
  repaired: string[];
  removed: string[];
  errors: string[];
}

export interface DiscoveredAccount {
  accountName: string;
  personaName: string | null;
  mostRecent: boolean;
}

export interface SandboxieInfo {
  installed: boolean;
  installDir: string | null;
  startExe: string | null;
  version: string | null;
}

export interface RunningGame {
  pid: number;
  exeName: string;
  exePath: string;
}

export interface RunningSandbox {
  login: string;
  boxName: string;
  startedAt: number;
  pids: number[];
}

export interface InstalledGame {
  appid: number;
  name: string;
  installdir: string;
  libraryPath: string;
  iconPath: string | null;
}

export type SandboxieDownloadPhase =
  | "resolving"
  | "downloading"
  | "installing"
  | "done"
  | "failed";

export interface SandboxieDownloadProgress {
  phase: SandboxieDownloadPhase;
  downloaded: number;
  total: number | null;
  percent: number | null;
  name: string | null;
}

export const SANDBOXIE_PROGRESS_EVENT = "sandboxie-download-progress";

export type LaunchOutcome =
  | { kind: "switch"; pid: number; previousAutologin: string | null }
  | { kind: "sandbox"; pid: number };

export const api = {
  detectMainSteam: () => invoke<MainSteamInfo>("detect_main_steam"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (login: string, display: string | null) =>
    invoke<Account>("add_account", { login, display }),
  removeAccount: (login: string, deleteFiles: boolean) =>
    invoke<void>("remove_account", { login, deleteFiles }),
  verifyAccount: (login: string) => invoke<AccountHealth>("verify_account", { login }),
  repairAccount: (login: string) => invoke<void>("repair_account", { login }),
  launchShadow: (login: string, mode?: LaunchMode) =>
    invoke<LaunchOutcome>("launch_shadow", { login, mode: mode ?? null }),
  changeWorkspace: (newPath: string, strategy: "Move" | "Relink" | "Cancel") =>
    invoke<void>("change_workspace", { newPath, strategy }),
  setWorkspaceInitial: (newPath: string) =>
    invoke<void>("set_workspace_initial", { newPath }),
  setMainSteamOverride: (newPath: string | null) =>
    invoke<void>("set_main_steam_override", { newPath }),
  cleanupStaleJunctions: () => invoke<CleanupReport>("cleanup_stale_junctions"),
  discoverSteamAccounts: () => invoke<DiscoveredAccount[]>("discover_steam_accounts"),
  importDiscoveredAccounts: (logins: string[], personas: Record<string, string>) =>
    invoke<Account[]>("import_discovered_accounts", { logins, personas }),
  defaultWorkspace: () => invoke<string | null>("default_workspace"),
  detectSandboxie: () => invoke<SandboxieInfo>("detect_sandboxie"),
  installSandboxie: (installerPath: string) =>
    invoke<SandboxieInfo>("install_sandboxie", { installerPath }),
  downloadAndInstallSandboxie: () =>
    invoke<SandboxieInfo>("download_and_install_sandboxie"),
  listRunningGames: () => invoke<RunningGame[]>("list_running_games"),
  revertLastSwitch: () => invoke<void>("revert_last_switch"),
  closeWindow: () => invoke<void>("close_window"),
  minimizeWindow: () => invoke<void>("minimize_window"),
  startDrag: () => invoke<void>("start_drag"),
  isElevated: () => invoke<boolean>("is_elevated"),
  relaunchAsAdmin: () => invoke<void>("relaunch_as_admin"),
  setAccountFavorite: (login: string, value: boolean) =>
    invoke<void>("set_account_favorite", { login, value }),
  refreshAccountAvatar: (login: string) =>
    invoke<string | null>("refresh_account_avatar", { login }),
  listRunningSandboxes: () => invoke<RunningSandbox[]>("list_running_sandboxes"),
  stopSandbox: (login: string) => invoke<void>("stop_sandbox", { login }),
  listAccountGames: (login: string) =>
    invoke<InstalledGame[]>("list_account_games", { login }),
  launchGame: (login: string, appid: number, mode?: LaunchMode) =>
    invoke<LaunchOutcome>("launch_game", { login, appid, mode: mode ?? null }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  createAccountShortcut: (login: string) =>
    invoke<string>("create_account_shortcut", { login }),
};

export async function pickFolder(title: string): Promise<string | null> {
  const result = await open({ directory: true, multiple: false, title });
  if (typeof result === "string") return result;
  return null;
}

export async function pickFile(title: string, ext: string[]): Promise<string | null> {
  const result = await open({
    multiple: false,
    title,
    filters: [{ name: "File", extensions: ext }],
  });
  if (typeof result === "string") return result;
  return null;
}
