# Screenshots

The `*-v025.jpg` files show the actual v0.2.5 frontend with synthetic Tauri responses. No local Steam accounts, secrets or live transactions are used. The older images remain as historical assets and are not displayed in the project README.

## Reproduce

```powershell
npm ci
npm run build
python tests/ui/serve.py
```

Open `http://127.0.0.1:18743/showcase.html`. The iframe matches the native window's default size from `src-tauri/tauri.conf.json`: **1000 × 720**. Capture just the app frame, without the surrounding browser or fixture caption.

1. **`accounts-v025.jpg`** — Accounts view, cards mode, Russian locale. Six synthetic accounts show avatars, inline Steam Guard codes, launch history and an active sandbox.
2. **`confirmations-v025.jpg`** — Confirmations view; filter to `SHADOW · @demo_main`, expand the trade with **Предметы**, and wait for item images to load. Leave every operation unselected.
3. **`settings-v025.jpg`** — Settings view at the top, showing Appearance. Use the defaults: glass density 56%, particles on, brightness 72%, animation on.

Use the rendered UI without retouching or adding controls. The fixture remains interactive; codes count down and particles move, so captures are not pixel deterministic. Item images require access to Steam's public CDN. See `tests/ui/mock.js` for the synthetic data and `tests/ui/showcase.html` for the frame.
