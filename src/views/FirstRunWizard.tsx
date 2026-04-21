import { useEffect, useState } from "react";
import { api, pickFolder, pickFile } from "../api/tauri";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";

const ASCII = String.raw` ___ _  _   _   ___  _____      __
/ __| || | /_\ |   \/ _ \ \    / /
\__ \ __ |/ _ \| |) | (_) \ \/\/ /
|___/_||_/_/ \_\___/ \___/ \_/\_/`;

export function FirstRunWizard() {
  const { settings, mainSteam, mainSteamError } = useApp();
  const { t } = useI18n();
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      if (settings?.workspace) {
        setWorkspace(settings.workspace);
      } else {
        try {
          const def = await api.defaultWorkspace();
          if (def) setWorkspace(def);
        } catch {}
      }
    })();
  }, [settings]);

  const pickWorkspace = async () => {
    const p = await pickFolder(t("wizard.pickFolder"));
    if (p) setWorkspace(p);
  };

  const useDefault = async () => {
    const def = await api.defaultWorkspace();
    if (def) setWorkspace(def);
  };

  const pickSteam = async () => {
    const p = await pickFile("steam.exe", ["exe"]);
    if (p) {
      const dir = p.replace(/\\steam\.exe$/i, "");
      await api.setMainSteamOverride(dir);
      await useApp.getState().bootstrap();
    }
  };

  const complete = async () => {
    if (!workspace) {
      setErr(t("common.unknown"));
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await api.setWorkspaceInitial(workspace);
      await useApp.getState().bootstrap();
      // Only prompt for import if the workspace is fresh (no existing accounts).
      const existing = useApp.getState().accounts;
      if (existing.length === 0) {
        useApp.getState().triggerImportPrompt();
      }
    } catch (e: any) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="wizard">
      <pre className="ascii">{ASCII}</pre>

      <div className="step">
        <h3>{t("wizard.step1")}</h3>
        {mainSteam ? (
          <>
            <div className="path">{mainSteam.installDir}</div>
            <div className="hint">
              {t("wizard.steamHint")}
              {mainSteam.autologinUser || t("common.none")}
            </div>
          </>
        ) : (
          <>
            <div className="err-banner">
              {t("wizard.steamNotFound")}
            </div>
            <div style={{ marginTop: 4 }}>
              <button
                className="xs"
                onClick={() => api.openUrl("https://store.steampowered.com/about/")}
              >
                {t("wizard.installSteam")}
              </button>
            </div>
            {mainSteamError && mainSteamError !== "STEAM_NOT_FOUND" && (
              <div className="hint" style={{ marginTop: 4, opacity: 0.7 }}>
                {mainSteamError}
              </div>
            )}
          </>
        )}
        <div style={{ marginTop: 4 }}>
          <button className="xs" onClick={pickSteam}>
            {t("wizard.override")}
          </button>
        </div>
      </div>

      <div className="step">
        <h3>{t("wizard.step2")}</h3>
        <div className="hint">{t("wizard.workspaceHint")}</div>
        <div className="path">{workspace || t("common.unknown")}</div>
        <div style={{ marginTop: 4 }} className="flex">
          <button className="xs" onClick={pickWorkspace}>
            {t("wizard.pickFolder")}
          </button>
          <button className="xs ghost" onClick={useDefault}>
            {t("wizard.useDefault")}
          </button>
        </div>
      </div>

      <div className="step">
        <h3>{t("wizard.step3")}</h3>
        <div className="hint">
          {t("wizard.confirmHint")}{" "}
          <code>&lt;ws&gt;/accounts/&lt;login&gt;/steamapps → {mainSteam?.steamappsDir || "…"}</code>
        </div>
      </div>

      {err && <div className="err-banner">{err}</div>}

      <div className="flex" style={{ marginTop: 4 }}>
        <div className="spacer" />
        <button
          className="primary"
          disabled={busy || !mainSteam || !workspace}
          onClick={complete}
        >
          {busy ? t("wizard.saving") : t("wizard.start")}
        </button>
      </div>
    </div>
  );
}
