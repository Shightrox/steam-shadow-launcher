import { useAppearance } from "../state/appearance";
import { useI18n } from "../i18n";

export function AppearanceSettings() {
  const { t } = useI18n();
  const appearance = useAppearance();
  return <section className="card compact appearance-settings">
    <h2 className="title">{t("appearance.title")}</h2>
    <label><span>{t("appearance.glass")}</span><input type="range" min="40" max="80" value={appearance.glassDensity} onChange={e => appearance.update({ glassDensity: Number(e.target.value) })} /><output>{appearance.glassDensity}%</output></label>
    <label><span>{t("appearance.particles")}</span><input type="checkbox" checked={appearance.particles} onChange={e => appearance.update({ particles: e.target.checked })} /></label>
    <label><span>{t("appearance.brightness")}</span><input type="range" min="25" max="100" disabled={!appearance.particles} value={appearance.particleStrength} onChange={e => appearance.update({ particleStrength: Number(e.target.value) })} /><output>{appearance.particleStrength}%</output></label>
    <label><span>{t("appearance.motion")}</span><input type="checkbox" checked={appearance.motion} onChange={e => appearance.update({ motion: e.target.checked })} /></label>
  </section>;
}
