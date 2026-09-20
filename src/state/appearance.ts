import { create } from "zustand";
import { persist } from "zustand/middleware";

interface Appearance {
  glassDensity: number;
  particles: boolean;
  particleStrength: number;
  motion: boolean;
}

export const useAppearance = create<Appearance & { update(value: Partial<Appearance>): void }>()(
  persist((set) => ({
    glassDensity: 56,
    particles: true,
    particleStrength: 72,
    motion: true,
    update: value => set(value),
  }), { name: "shadow.appearance" }),
);
