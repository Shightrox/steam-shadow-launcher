import type { AccountHealth } from "../api/tauri";
import { useI18n } from "../i18n";

export function HealthBadge({ health }: { health?: AccountHealth }) {
  const { t } = useI18n();
  if (!health) return <span className="badge">...</span>;
  if (health.ready) return <span className="badge ok">{t("health.ready")}</span>;
  const k = health.junction.kind;
  if (k === "missing") return <span className="badge warn">{t("health.junctionMissing")}</span>;
  if (k === "stale") return <span className="badge warn">{t("health.junctionStale")}</span>;
  if (k === "notajunction") return <span className="badge err">{t("health.notJunction")}</span>;
  if (!health.configDirExists) return <span className="badge warn">{t("health.noConfig")}</span>;
  return <span className="badge warn">{t("health.notReady")}</span>;
}
