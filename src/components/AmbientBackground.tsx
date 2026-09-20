import { useEffect, useRef, type CSSProperties } from "react";
import { useAppearance } from "../state/appearance";

const particles = Array.from({ length: 30 }, (_, index) => {
  const far = index % 3 === 0;
  const frame = index % 8 === 5;
  return {
    far, frame,
    style: {
      "--px": `${(index * 37 + 11) % 98 + 1}%`,
      "--py": `${(index * 23 + 7) % 96 + 2}%`,
      "--ps": `${frame ? 4 : far ? 1 : 2}px`,
      "--pd": `${far ? 46 + index % 13 : 22 + index % 17}s`,
      "--delay": `${-index * 4.7}s`,
      "--dx": `${(index % 2 ? 1 : -1) * (12 + index % 21)}px`,
    } as CSSProperties,
  };
});

export function AmbientBackground() {
  const { glassDensity, particles: enabled, particleStrength, motion } = useAppearance();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const page = document.documentElement;
    page.style.setProperty("--glass-alpha", String(glassDensity / 100));
    page.style.setProperty("--particle-strength", String(particleStrength / 100));
    page.dataset.motion = String(motion);
  }, [glassDensity, particleStrength, motion]);

  useEffect(() => {
    const layer = ref.current;
    if (!layer) return;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)");
    let visible = true;
    const sync = () => {
      layer.dataset.running = String(enabled && motion && !reduced.matches && !document.hidden && visible);
    };
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; sync(); });
    observer.observe(layer);
    document.addEventListener("visibilitychange", sync);
    reduced.addEventListener("change", sync);
    sync();
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", sync);
      reduced.removeEventListener("change", sync);
    };
  }, [enabled, motion]);

  return <div ref={ref} className="ambient-background" aria-hidden="true" hidden={!enabled}>
    {particles.map((particle, index) => <span key={index} className="ambient-particle" data-depth={particle.far ? "far" : "near"} data-shape={particle.frame ? "frame" : "dot"} style={particle.style} />)}
  </div>;
}
