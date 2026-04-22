import { useEffect, useState } from "react";
import { api, type UpdateInfo } from "../api/tauri";
import { useI18n } from "../i18n";
import { Spinner } from "./Spinner";
import { ErrorBox } from "./ErrorBox";

interface Props {
  info: UpdateInfo;
  onDismiss(): void;
}

/// Update prompt. Shows the latest version + release notes and offers an
/// in-place "update now" button. The install flow is:
///
///   1. The backend downloads the new portable exe next to the current one.
///   2. The live exe is renamed to `*.old` and the fresh build promoted.
///   3. A detached .cmd waits for our PID to die, cleans `*.old`, and
///      relaunches the new build.
///
/// Once the user clicks "Update now" we show a spinner and let Rust do the
/// heavy lifting. Our process is terminated ~300 ms after step 1 finishes.
export function UpdateModal({ info, onDismiss }: Props) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState<"prompt" | "installing" | "restarting">(
    "prompt",
  );
  const [error, setError] = useState<string | null>(null);

  // Swallow backdrop / Esc closes while busy — we don't want the user to
  // click away while the exe is mid-rename.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onDismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onDismiss]);

  const doUpdate = async () => {
    if (!info.download_url) {
      setError(t("update.noAsset"));
      return;
    }
    setBusy(true);
    setError(null);
    setPhase("installing");
    try {
      await api.applyUpdate(info.download_url);
      // If we're still alive past apply_update, Rust spawned the relaunch
      // script and is about to exit. Flip to a "restarting" screen so the
      // last thing the user sees isn't a frozen modal.
      setPhase("restarting");
    } catch (e: any) {
      setError(String(e));
      setPhase("prompt");
      setBusy(false);
    }
  };

  return (
    <div
      className="modal-backdrop"
      onClick={() => !busy && onDismiss()}
    >
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ minWidth: 480, maxWidth: 560 }}
      >
        <div className="modal-title">
          ⬆ {t("update.title", { version: info.latest })}
        </div>
        <div className="modal-body">
          {phase === "prompt" && (
            <>
              <div className="hint" style={{ marginBottom: 8 }}>
                {t("update.summary", {
                  current: info.current,
                  latest: info.latest,
                })}
              </div>
              {info.notes && (
                <div
                  style={{
                    fontFamily: "var(--mono)",
                    fontSize: 11,
                    lineHeight: 1.55,
                    color: "var(--fg-dim)",
                    background: "var(--bg-elev)",
                    border: "1px solid var(--border)",
                    padding: 10,
                    maxHeight: 220,
                    overflow: "auto",
                    whiteSpace: "pre-wrap",
                    marginTop: 4,
                  }}
                >
                  {info.notes}
                </div>
              )}
              {!info.download_url && (
                <div className="auth-warn" style={{ marginTop: 12 }}>
                  ⚠ {t("update.noAsset")}
                </div>
              )}
              {error && <ErrorBox message={error} />}
              <div className="modal-actions">
                <button
                  className="xs"
                  onClick={() => api.openUrl(info.release_url)}
                >
                  ↗ {t("update.viewOnGithub")}
                </button>
                <button className="xs ghost" onClick={onDismiss}>
                  {t("update.later")}
                </button>
                <button
                  className="primary"
                  disabled={!info.download_url}
                  onClick={doUpdate}
                >
                  {t("update.install")}
                </button>
              </div>
            </>
          )}

          {phase === "installing" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("update.downloading")}</div>
              <div className="hint">{info.asset_name}</div>
            </div>
          )}

          {phase === "restarting" && (
            <div className="auth-loading">
              <Spinner size="md" />
              <div>{t("update.restarting")}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
