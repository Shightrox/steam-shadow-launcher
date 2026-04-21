import { useApp } from "../state/store";

export function ToastHost() {
  const { toasts, dismissToast } = useApp();
  if (toasts.length === 0) return null;
  return (
    <div className="toast-host">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast toast-${t.kind}`}
          onClick={() => dismissToast(t.id)}
          role="alert"
        >
          <span className="toast-icon" aria-hidden="true">
            {t.kind === "error" ? "!" : t.kind === "success" ? "✓" : "·"}
          </span>
          <span className="toast-msg">{t.msg}</span>
        </div>
      ))}
    </div>
  );
}
