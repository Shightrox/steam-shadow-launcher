import assert from "node:assert/strict";
import { test } from "node:test";
import { build } from "esbuild";

// Exercise the real store and clock logic with an IPC stub. No Steam requests.
const mock = {};
globalThis.__shadowTestApi = mock;
globalThis.window = { setTimeout, clearTimeout, setInterval, clearInterval };
const bundled = await build({
  stdin: { contents: 'export { useApp } from "./src/state/store"; export { remainingSeconds, copyGuardCode } from "./src/state/guardCode"; export { steamImageUrl } from "./src/state/steamImage";', resolveDir: process.cwd() },
  bundle: true, write: false, platform: "node", format: "esm",
  plugins: [{ name: "fake-ipc", setup(b) {
    b.onResolve({ filter: /api\/tauri$/ }, () => ({ path: "ipc", namespace: "test" }));
    b.onLoad({ filter: /.*/, namespace: "test" }, () => ({ contents: 'export const api = new Proxy({}, { get: (_, name) => (...args) => globalThis.__shadowTestApi[name](...args) });' }));
  } }],
});
const { useApp, remainingSeconds, copyGuardCode, steamImageUrl } = await import(`data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`);
const reset = () => {
  for (const key of Object.keys(mock)) delete mock[key];
  useApp.setState(s => ({ workspaceEpoch: s.workspaceEpoch + 1, settings: null, authLock: { enabled: false, unlocked: false, hasEncryptedFiles: false }, codes: {}, confirmations: {}, confErrors: {}, confLoading: {}, authStatus: {}, sessionStates: {}, accounts: [], toast() {}, log() {} }));
};
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };

test("OTP expires at the issued deadline, including after a long UI pause", () => {
  const code = { code: "ABCDE", generatedAt: 1000, periodRemaining: 7 };
  assert.equal(remainingSeconds(code, 1000500), 7);
  assert.equal(remainingSeconds(code, 1006999), 1);
  assert.equal(remainingSeconds(code, 1007000), 0);
  assert.equal(remainingSeconds(code, 1100000), 0);
});
test("requests for an OTP share one in-flight call and discard another workspace's result", async () => {
  reset(); const pending = deferred(); let calls = 0;
  mock.authGenerateCode = () => { calls++; return pending.promise; };
  const first = useApp.getState().refreshCode("demo");
  const second = useApp.getState().refreshCode("demo");
  assert.equal(calls, 1);
  useApp.setState(s => ({ workspaceEpoch: s.workspaceEpoch + 1 }));
  pending.resolve({ code: "OLDWS", generatedAt: Date.now() / 1000, periodRemaining: 30 });
  await Promise.all([first, second]);
  assert.deepEqual(useApp.getState().codes, {});
});
test("confirmation responses cannot repopulate a changed workspace", async () => {
  reset(); const pending = deferred();
  mock.authConfirmationsList = () => pending.promise;
  const request = useApp.getState().refreshConfirmations("demo");
  useApp.setState(s => ({ workspaceEpoch: s.workspaceEpoch + 1, confLoading: {} }));
  pending.resolve([{ id: "belongs-to-old-workspace" }]); await request;
  assert.deepEqual(useApp.getState().confirmations, {});
  assert.deepEqual(useApp.getState().confLoading, {});
});
test("locking clears secrets immediately and rejects an older in-flight OTP", async () => {
  reset(); const pending = deferred(); const lock = deferred();
  useApp.setState({ authLock: { enabled: true, unlocked: true, hasEncryptedFiles: true } });
  mock.authGenerateCode = () => pending.promise;
  mock.authLock = () => lock.promise;
  mock.authLockStatus = async () => ({ enabled: true, unlocked: false, hasEncryptedFiles: true });
  const code = useApp.getState().refreshCode("demo");
  const locking = useApp.getState().lockAuth();
  assert.equal(useApp.getState().authLock.unlocked, false);
  pending.resolve({ code: "STALE", generatedAt: Date.now() / 1000, periodRemaining: 30 });
  lock.resolve(); await Promise.all([code, locking]);
  assert.deepEqual(useApp.getState().codes, {});
});
test("copy obtains a current code and never copies an expired cached code", async () => {
  reset(); let copied = null;
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText: async value => { copied = value; } } } });
  useApp.setState({ codes: { demo: { code: "STALE", generatedAt: 1, periodRemaining: 30 } } });
  mock.authGenerateCode = async () => ({ code: "FRESH", generatedAt: Date.now() / 1000, periodRemaining: 30 });
  await copyGuardCode("demo"); assert.equal(copied, "FRESH");
  copied = null;
  mock.authGenerateCode = async () => ({ code: "OLD", generatedAt: 1, periodRemaining: 30 });
  await assert.rejects(copyGuardCode("demo"), /AUTH_CODE_UNAVAILABLE/);
  assert.equal(copied, null);
});
test("bad settings produce a recoverable boot error", async () => {
  reset(); mock.getSettings = async () => { throw new Error("invalid settings JSON"); };
  await useApp.getState().bootstrap();
  assert.match(useApp.getState().bootError, /invalid settings JSON/);
  assert.equal(useApp.getState().settings, null);
});

test("Steam thumbnails accept CDN variants and reject non-Steam or privileged URLs", () => {
  assert.equal(steamImageUrl("http://community.steamstatic.com/economy/image/demo"), "https://community.steamstatic.com/economy/image/demo");
  assert.equal(steamImageUrl("//avatars.steamstatic.com/demo.jpg"), "https://avatars.steamstatic.com/demo.jpg");
  assert.equal(steamImageUrl("https://steamcommunity-a.akamaihd.net/economy/image/demo"), "https://steamcommunity-a.akamaihd.net/economy/image/demo");
  for (const url of [undefined, "", "javascript:alert(1)", "file:///C:/secret", "data:image/svg+xml,<svg/>", "https://steamstatic.com.attacker.test/a", "https://attackersteamstatic.com/a", "https://user:secret@community.steamstatic.com/a", "https://127.0.0.1/a", "https://community.steamstatic.com:8080/a"]) {
    assert.equal(steamImageUrl(url), null, url);
  }
});
