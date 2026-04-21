import { useEffect, useRef } from "react";

export interface ContextMenuItem {
  label: string;
  onClick(): void;
  disabled?: boolean;
  danger?: boolean;
  divider?: never;
}
export interface ContextMenuDivider {
  divider: true;
  label?: never;
  onClick?: never;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuDivider;

interface Props {
  x: number;
  y: number;
  items: ContextMenuEntry[];
  onClose(): void;
}

export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", handler);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", handler);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);

  // Clamp to viewport.
  const maxX = window.innerWidth - 200;
  const maxY = window.innerHeight - items.length * 22 - 8;
  const px = Math.min(x, maxX);
  const py = Math.min(y, maxY);

  return (
    <div
      ref={ref}
      className="ctx-menu"
      style={{ left: px, top: py }}
      onClick={(e) => e.stopPropagation()}
    >
      {items.map((it, i) =>
        "divider" in it ? (
          <div key={i} className="ctx-divider" />
        ) : (
          <button
            key={i}
            className={"ctx-item" + (it.danger ? " danger" : "")}
            disabled={it.disabled}
            onClick={() => {
              it.onClick();
              onClose();
            }}
          >
            {it.label}
          </button>
        )
      )}
    </div>
  );
}
