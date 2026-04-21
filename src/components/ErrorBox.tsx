import { useApp } from "../state/store";
import { useI18n } from "../i18n";

/// Inline error box with a small "copy" button in the bottom-right corner.
/// Used inside auth/login modals where errors can be wall-of-text protobuf
/// dumps that are useless without copy-paste.
export function ErrorBox({ message }: { message: string }) {
  const { toast } = useApp();
  const { t } = useI18n();
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(message);
      toast("success", t("auth.copied"));
    } catch (e: any) {
      toast("error", String(e));
    }
  };
  return (
    <div className="auth-err err-box">
      <div className="err-box-text">{message}</div>
      <button
        className="err-box-copy"
        onClick={onCopy}
        title={t("auth.copy")}
        aria-label={t("auth.copy")}
      >
        ⎘
      </button>
    </div>
  );
}
