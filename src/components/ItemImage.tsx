import { useState } from "react";
import { steamImageUrl } from "../state/steamImage";
import { useI18n } from "../i18n";
import { Icon } from "./Icon";

export function ItemImage({ src, name, avatar = false }: { src?: string; name: string; avatar?: boolean }) {
  const { t } = useI18n();
  const url = steamImageUrl(src);
  const [failed, setFailed] = useState<string | null>(null);
  const visible = url && failed !== url;
  return <span className={`item-image${avatar ? " partner-avatar" : ""}`}>
    {visible ? <img src={url} alt={name} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(url)} />
      : <span role="img" aria-label={t("design.noImage")} title={t("design.noImage")}><Icon name={avatar ? "accounts" : "box"} size={26} /></span>}
  </span>;
}
