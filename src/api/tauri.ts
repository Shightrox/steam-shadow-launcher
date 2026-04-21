import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

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
  authMasterPasswordEnabled: boolean;
  authPollerEnabled: boolean;
  authPollerInterval: number;
  authAutoConfirmTrades: boolean;
  authAutoConfirmMarket: boolean;
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
  hasAuthenticator: boolean;
  authenticatorImportedAt: string | null;
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

export interface PollerConfig {
  enabled: boolean;
  interval: number;
  autoConfirmTrades: boolean;
  autoConfirmMarket: boolean;
}

export const SANDBOXIE_PROGRESS_EVENT = "sandboxie-download-progress";
export const AUTH_CONFIRMS_EVENT = "auth://confirmations-changed";
export const AUTH_AUTO_CONFIRMED_EVENT = "auth://auto-confirmed";

export type LaunchOutcome =
  | { kind: "switch"; pid: number; previousAutologin: string | null }
  | { kind: "sandbox"; pid: number };

export interface AccountAuthStatus {
  login: string;
  hasAuthenticator: boolean;
  accountName: string | null;
  importedAt: string | null;
}

export interface GuardCode {
  code: string;
  generatedAt: number;
  periodRemaining: number;
}

export interface Confirmation {
  id: string;
  nonce: string;
  creator_id: string;
  headline: string;
  summary: string[];
  type: number;
  type_name: string;
  accept: string;
  cancel: string;
  icon: string;
}

export interface RespondResult {
  id: string;
  success: boolean;
  message: string;
}

export interface AuthLockStatus {
  enabled: boolean;
  unlocked: boolean;
  hasEncryptedFiles: boolean;
}

export type ConfirmOp = "allow" | "reject";

export interface AllowedConfirmation {
  confirmation_type: number;
  associated_message: string;
}

export interface BeginOutcome {
  clientId: string;
  requestId: string;
  steamId: string;
  weakToken: string;
  allowedConfirmations: AllowedConfirmation[];
  interval: number;
  extendedDomain?: string | null;
}

export type PollState =
  | { state: "Pending" }
  | { state: "NeedsCode" }
  | { state: "Failed"; reason: string }
  | {
      state: "Done";
      accessToken: string;
      refreshToken: string;
      accountName: string;
      steamId: string;
      newGuardData?: string | null;
    };

// ── P12: AddAuthenticator wizard ────────────────────────────────────────

export interface SetPhoneResult {
  confirmation_email_address: string;
  phone_number_formatted: string;
}

export interface PhoneState {
  awaiting_email: boolean;
  seconds_to_wait: number;
}

export interface AddCreatePublic {
  phone_number_hint: string;
  server_time: string;
}

export interface AddFinalizePublic {
  success: boolean;
  want_more: boolean;
  status: number;
  revocation_code: string | null;
}

export interface AddDiagnostic {
  /** "none" | "email" | "mobile" */
  guard: "none" | "email" | "mobile";
  already_has_mobile: boolean;
  phone_attached: boolean;
  /** May be empty. */
  phone_hint: string;
  /** "phone-required" | "no-phone-fast" | "blocker-no-guard" | "blocker-already-mobile" */
  suggested_path:
    | "phone-required"
    | "no-phone-fast"
    | "blocker-no-guard"
    | "blocker-already-mobile";
}

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
  authOpenFolder: (login: string) =>
    invoke<string>("auth_open_folder", { login }),
  // ── P11: Authenticator ───────────────────────────────────────────────
  authStatus: () => invoke<AccountAuthStatus[]>("auth_status"),
  authImportMafile: (login: string, source: string, encryptionPassword?: string) =>
    invoke<AccountAuthStatus>("auth_import_mafile", {
      login,
      source,
      encryptionPassword: encryptionPassword ?? null,
    }),
  authExportMafile: (login: string, targetPath: string) =>
    invoke<void>("auth_export_mafile", { login, targetPath }),
  authRemove: (login: string) => invoke<void>("auth_remove", { login }),
  authGenerateCode: (login: string) =>
    invoke<GuardCode>("auth_generate_code", { login }),
  authSyncTime: () => invoke<void>("auth_sync_time"),
  authConfirmationsList: (login: string) =>
    invoke<Confirmation[]>("auth_confirmations_list", { login }),
  authConfirmationsRespond: (login: string, ids: string[], op: ConfirmOp) =>
    invoke<RespondResult[]>("auth_confirmations_respond", { login, ids, op }),
  authLoginBegin: (accountName: string, password: string) =>
    invoke<BeginOutcome>("auth_login_begin", { accountName, password }),
  authLoginSubmitCode: (clientId: string, steamId: string, code: string, codeType: number) =>
    invoke<void>("auth_login_submit_code", { clientId, steamId, code, codeType }),
  authLoginPoll: (login: string, clientId: string, requestId: string, allowedConfirmations?: number[]) =>
    invoke<PollState>("auth_login_poll", { login, clientId, requestId, allowedConfirmations }),
  authLoginRefresh: (login: string) =>
    invoke<void>("auth_login_refresh", { login }),
  authLockStatus: () => invoke<AuthLockStatus>("auth_lock_status"),
  authUnlock: (password: string) => invoke<void>("auth_unlock", { password }),
  authLock: () => invoke<void>("auth_lock"),
  authSetMasterPassword: (oldPassword: string | null, newPassword: string | null) =>
    invoke<void>("auth_set_master_password", { oldPassword, newPassword }),
  authPollerConfigure: (cfg: PollerConfig) =>
    invoke<void>("auth_poller_configure", { cfg }),
  authPollerPoke: () => invoke<void>("auth_poller_poke"),
  // ── P12: AddAuthenticator wizard ───────────────────────────────────
  authAddSetPhone: (login: string, phoneNumber: string, phoneCountryCode: string) =>
    invoke<SetPhoneResult>("auth_add_set_phone", {
      login,
      phoneNumber,
      phoneCountryCode,
    }),
  authAddCheckEmail: (login: string) =>
    invoke<PhoneState>("auth_add_check_email", { login }),
  authAddSendSms: (login: string) => invoke<void>("auth_add_send_sms", { login }),
  authAddVerifyPhone: (login: string, code: string) =>
    invoke<void>("auth_add_verify_phone", { login, code }),
  authAddCreate: (login: string) =>
    invoke<AddCreatePublic>("auth_add_create", { login }),
  authAddFinalize: (login: string, smsCode: string, tryNumber: number, validateSms: boolean) =>
    invoke<AddFinalizePublic>("auth_add_finalize", {
      login,
      smsCode,
      tryNumber,
      validateSms,
    }),
  authAddPersist: (login: string) => invoke<void>("auth_add_persist", { login }),
  authAddCancel: (login: string) => invoke<void>("auth_add_cancel", { login }),
  authAddDiagnose: (login: string) =>
    invoke<AddDiagnostic>("auth_add_diagnose", { login }),
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

export async function pickSaveFile(
  title: string,
  defaultName: string,
  ext: string[],
): Promise<string | null> {
  const result = await save({
    title,
    defaultPath: defaultName,
    filters: [{ name: "File", extensions: ext }],
  });
  if (typeof result === "string") return result;
  return null;
}
