import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "../i18n";
import { useApp } from "../state/store";
import { Spinner } from "./Spinner";
import {
  api,
  SANDBOXIE_PROGRESS_EVENT,
  type SandboxieDownloadProgress,
} from "../api/tauri";

interface Props {
  open: boolean;
  onClose(): void;
  onInstalled?(): void;
}

const fmtMB = (bytes: number) => (bytes / (1024 * 1024)).toFixed(1);

export function SandboxieInstallModal({ open, onClose, onInstalled }: Props) {
  const { t } = useI18n();
  const downloadAndInstall = useApp((s) => s.downloadAndInstallSandboxie);
  const [stage, setStage] = useState<
    "checkingAdmin" | "needAdmin" | "confirm" | "running" | "done" | "failed"
  >("checkingAdmin");
  const [progress, setProgress] = useState<SandboxieDownloadProgress | null>(null);
  const [errMsg, setErrMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setStage("checkingAdmin");
    setProgress(null);
    setErrMsg(null);
    (async () => {
      try {
        const elevated = await api.isElevated();
        setStage(elevated ? "confirm" : "needAdmin");
      } catch {
        setStage("confirm");
      }
    })();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let un: UnlistenFn | null = null;
    listen<SandboxieDownloadProgress>(SANDBOXIE_PROGRESS_EVENT, (e) => {
      setProgress(e.payload);
    }).then((fn) => {
      un = fn;
    });
    return () => {
      if (un) un();
    };
  }, [open]);

  if (!open) return null;

  const start = async () => {
    setStage("running");
    setProgress({ phase: "resolving", downloaded: 0, total: null, percent: null, name: null });
    try {
      const ok = await downloadAndInstall();
      if (ok) {
        setStage("done");
        onInstalled?.();
      } else {
        setStage("failed");
        setErrMsg(t("sb.install.failed"));
      }
    } catch (e: any) {
      setStage("failed");
      setErrMsg(String(e));
    }
  };

  const renderProgress = () => {
    if (!progress) return null;
    const phaseLabel =
      progress.phase === "resolving"
        ? t("sb.install.resolving")
        : progress.phase === "downloading"
        ? t("sb.install.downloading")
        : progress.phase === "installing"
        ? t("sb.install.installing")
        : progress.phase === "done"
        ? t("sb.install.done")
        : t("sb.install.failed");
    const pct = progress.percent ?? 0;
    const showBar = progress.phase === "downloading";
    return (
      <div className="install-progress">
        <div className="sub">{phaseLabel}{progress.name ? ` — ${progress.name}` : ""}</div>
        {showBar && (
          <>
            <div className="progress-bar">
              <div className="progress-fill" style={{ width: `${pct}%` }} />
              <div className="progress-text">{pct}%</div>
            </div>
            {progress.total != null && (
              <div className="sub">
                {t("sb.install.downloaded", {
                  mb: fmtMB(progress.downloaded),
                  total: fmtMB(progress.total),
                })}
              </div>
            )}
          </>
        )}
        {progress.phase === "installing" && (
          <div className="progress-bar indeterminate">
            <div className="progress-fill" />
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="modal-overlay">
      <div className="modal">
        <h2>{t("sb.install.confirmTitle")}</h2>
        {stage === "checkingAdmin" && (
          <div className="sub" style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <Spinner size="sm" inline /> {t("common.loading")}
          </div>
        )}
        {stage === "needAdmin" && (
          <>
            <div className="sub" style={{ marginBottom: 8 }}>
              {t("sb.install.needAdmin")}
            </div>
            <div className="buttons">
              <button className="ghost" onClick={onClose}>
                {t("common.cancel")}
              </button>
              <button
                className="primary"
                onClick={async () => {
                  try {
                    await api.relaunchAsAdmin();
                  } catch (e: any) {
                    setErrMsg(String(e));
                    setStage("failed");
                  }
                }}
              >
                {t("sb.install.relaunch")}
              </button>
            </div>
          </>
        )}
        {stage === "confirm" && (
          <>
            <div className="sub" style={{ marginBottom: 8 }}>
              {t("sb.install.confirmBody")}
            </div>
            <div className="buttons">
              <button className="ghost" onClick={onClose}>
                {t("sb.install.no")}
              </button>
              <button className="primary" onClick={start}>
                {t("sb.install.yes")}
              </button>
            </div>
          </>
        )}
        {stage === "running" && renderProgress()}
        {stage === "done" && (
          <>
            <div className="sub" style={{ marginBottom: 8, color: "var(--ok)" }}>
              {t("sb.install.done")}
            </div>
            <div className="buttons">
              <button className="primary" onClick={onClose}>
                {t("common.close")}
              </button>
            </div>
          </>
        )}
        {stage === "failed" && (
          <>
            <div className="sub" style={{ marginBottom: 8, color: "var(--danger)" }}>
              {errMsg || t("sb.install.failed")}
            </div>
            <div className="buttons">
              <button className="ghost" onClick={onClose}>
                {t("common.close")}
              </button>
              <button onClick={start}>{t("sb.install.retry")}</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
