import { useEffect, useState } from "react";
import type { GuardCode } from "../api/tauri";
import { useApp } from "./store";

export function remainingSeconds(code: GuardCode | undefined, now = Date.now()) {
  if (!code) return 0;
  return Math.max(0, Math.min(30, Math.ceil(code.generatedAt + code.periodRemaining - now / 1000)));
}

export function useGuardCode(login: string | null) {
  const code = useApp((s) => login ? s.codes[login] : undefined);
  const locked = useApp((s) => !!s.authLock?.enabled && !s.authLock.unlocked);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(tick);
  }, []);
  const remaining = remainingSeconds(code, now);
  useEffect(() => {
    if (login && !locked && remaining === 0) void useApp.getState().refreshCode(login);
  }, [login, locked, remaining, Math.floor(now / 1000)]);
  return { code: remaining > 0 && !locked ? code : undefined, remaining };
}

export async function copyGuardCode(login: string) {
  const epoch = useApp.getState().workspaceEpoch;
  const code = await useApp.getState().refreshCode(login);
  if (!code || epoch !== useApp.getState().workspaceEpoch || remainingSeconds(code) <= 0) {
    throw new Error("AUTH_CODE_UNAVAILABLE");
  }
  await navigator.clipboard.writeText(code.code);
}
