import { useApp } from "../state/store";
import { useI18n } from "../i18n";

export function LogDrawer({ open, onClose }: { open: boolean; onClose(): void }) {
  const logs = useApp((s) => s.logs);
  const { t } = useI18n();
  return (
    <div className={`log-drawer ${open ? "open" : ""}`}>
      <div className="hdr">
        <span>:: {t("common.log").toUpperCase()}</span>
        <button className="xs ghost" onClick={onClose}>x</button>
      </div>
      <ul>
        {logs.length === 0 && <li>—</li>}
        {logs.map((l, i) => (
          <li key={i} className={l.level}>
            [{new Date(l.ts).toLocaleTimeString()}] {l.msg}
          </li>
        ))}
      </ul>
    </div>
  );
}
