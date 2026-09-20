import { useState } from "react";
import { useApp } from "../state/store";
import { useI18n } from "../i18n";
import { SandboxieInstallModal } from "./SandboxieInstallModal";

export function LaunchModeSwitch() {
  const { t } = useI18n();
  const mode = useApp(s => s.settings?.defaultLaunchMode ?? "switch");
  const sandboxie = useApp(s => s.sandboxie);
  const setMode = useApp(s => s.setDefaultMode);
  const [installOpen, setInstallOpen] = useState(false);
  return <><div className="mode-switch" data-mode={mode} role="group" aria-label={t("sb.mode")}>
    <button aria-pressed={mode === "switch"} title={t("mode.switch.tip")} onClick={() => void setMode("switch")}>{t("sb.switch")}</button>
    <button aria-pressed={mode === "sandbox"} title={t("mode.sandbox.tip")} onClick={() => sandboxie?.installed ? void setMode("sandbox") : setInstallOpen(true)}>{t("sb.sandbox")}</button>
  </div><SandboxieInstallModal open={installOpen} onClose={() => setInstallOpen(false)} onInstalled={async () => { await setMode("sandbox"); }} /></>;
}
