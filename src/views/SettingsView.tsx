import { useState } from "react";
import { api, pickFolder } from "../api/tauri";
import { useApp } from "../state/store";
import { WorkspaceChangeDialog } from "../components/WorkspaceChangeDialog";
import { SandboxieInstallModal } from "../components/SandboxieInstallModal";
import { AuthSecuritySection } from "../components/AuthSecuritySection";
import { AuthPollerSection } from "../components/AuthPollerSection";
import { useI18n } from "../i18n";

export function SettingsView({ onClose }: { onClose(): void }) {
  const { settings, mainSteam, sandboxie, sbStatus, log } = useApp();
  const { t } = useI18n();
  const [pendingNew, setPendingNew] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [installOpen, setInstallOpen] = useState(false);

  const changeWorkspace = async () => {
    const p = await pickFolder(t("settings.change"));
    if (!p) return;
    if (settings?.workspace && p === settings.workspace) return;
    setPendingNew(p);
  };

  const onStrategy = async (strategy: "Move" | "Relink" | "Cancel") => {
    const target = pendingNew;
    setPendingNew(null);
    if (!target || strategy === "Cancel") return;
    setBusy(true);
    try {
      await api.changeWorkspace(target, strategy);
      await api.cleanupStaleJunctions();
      await useApp.getState().bootstrap();
      log("info", `Workspace (${strategy}): ${target}`);
    } catch (e: any) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  const overrideSteam = async () => {
    const dir = await pickFolder(t("settings.overrideBtn"));
    if (!dir) return;
    await api.setMainSteamOverride(dir);
    await useApp.getState().bootstrap();
  };

  const resetOverride = async () => {
    await api.setMainSteamOverride(null);
    await useApp.getState().bootstrap();
  };

  const cleanup = async () => {
    setBusy(true);
    try {
      const r = await api.cleanupStaleJunctions();
      log("info", `Cleanup: rep=${r.repaired.length} err=${r.errors.length}`);
      await useApp.getState().refreshAccounts();
    } catch (e: any) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  const revert = async () => {
    setBusy(true);
    try {
      await api.revertLastSwitch();
      log("info", "Reverted last switch");
      await useApp.getState().bootstrap();
    } catch (e: any) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  const installSB = () => {
    setInstallOpen(true);
  };

  return (
    <div className="main">
      <div className="toolbar">
        <h1 className="section-title">{t("settings.title")}</h1>
        <div className="spacer" />
        <button className="xs" onClick={onClose}>{t("common.back")}</button>
      </div>

      <div className="card compact">
        <div className="title">{t("settings.workspace")}</div>
        <code className="path">{settings?.workspace || "(?)"}</code>
        <div className="actions">
          <button className="xs" onClick={changeWorkspace} disabled={busy}>
            {t("settings.change")}
          </button>
        </div>
      </div>

      <div className="card compact">
        <div className="title">{t("settings.steam")}</div>
        <div className="sub">{t("settings.override")} {settings?.mainSteamPathOverride || t("common.none")}</div>
        <div className="sub">{t("settings.detected")} {mainSteam?.installDir || "?"}</div>
        <div className="actions">
          <button className="xs" onClick={overrideSteam}>{t("settings.overrideBtn")}</button>
          <button className="xs ghost" onClick={resetOverride}>{t("settings.reset")}</button>
        </div>
      </div>

      <div className="card compact">
        <div className="title">{t("settings.sandbox")}</div>
        <div className="sub">
          {t("settings.sandboxInstalled")} {sandboxie?.installed ? "YES" : "NO"}
        </div>
        {sandboxie?.installDir && (
          <div className="sub">
            {t("settings.sandboxPath")} {sandboxie.installDir}
          </div>
        )}
        <div className="actions">
          <button className="xs" onClick={installSB} disabled={sbStatus === "installing"}>
            {sbStatus === "installing" ? t("settings.sandboxInstalling") : t("settings.sandboxInstall")}
          </button>
        </div>
      </div>

      <div className="card compact">
        <div className="title">{t("settings.revert")}</div>
        <div className="sub">{t("settings.revertHint")}</div>
        <div className="actions">
          <button className="xs danger" onClick={revert} disabled={busy}>
            {t("settings.revertBtn")}
          </button>
        </div>
      </div>

      <div className="card compact">
        <div className="title">{t("settings.maintenance")}</div>
        <div className="sub">{t("settings.maintenanceHint")}</div>
        <div className="actions">
          <button className="xs" onClick={cleanup} disabled={busy}>
            {t("settings.cleanup")}
          </button>
        </div>
      </div>

      <AuthSecuritySection />
      <AuthPollerSection />

      <WorkspaceChangeDialog
        open={!!pendingNew}
        oldPath={settings?.workspace || ""}
        newPath={pendingNew || ""}
        onChoose={onStrategy}
      />
      <SandboxieInstallModal
        open={installOpen}
        onClose={() => setInstallOpen(false)}
      />
    </div>
  );
}
