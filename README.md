# Steam Shadow Launcher

> A small, fast, pixel-art launcher for Steam that lets you switch between
> accounts in one click — or run multiple Steam sessions in parallel via
> Sandboxie-Plus, without re-downloading a single game.

[![License: MIT](https://img.shields.io/badge/license-MIT-66ffcc)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-1f8c6e)
![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-2fb58f)

🇬🇧 **English** · [🇷🇺 Русский](#-русский)

---

## ✨ What it does

- **Account switcher (SWITCH mode)** — closes current Steam, flips
  `loginusers.vdf` + `AutoLoginUser` to the chosen account, relaunches.
  Same idea as SAM / TcNo Account Switcher, but with a proper UI and proper
  backups.
- **Parallel sessions (SANDBOX mode)** — spawns a second `steam.exe`
  inside an isolated Sandboxie-Plus box. Both Steams run simultaneously,
  share the `steamapps` library (no re-download), and never collide on the
  Steam singleton mutex.
- **Game picker** — pulls covers from your local `librarycache`, lets you
  launch `steam://rungameid/<appid>` directly from the chosen account.
- **Per-account desktop shortcuts** — a `.lnk` that boots the launcher in
  headless mode and starts the chosen account in the default mode.
- **Avatars, favorites, launch counters, profile / inventory shortcuts.**
- **Chromeless pixel-art UI** — fixed 760×520, custom titlebar, EN/RU.
- **Doesn't touch your main Steam files** without making a backup first.

## 🖼️ Screenshots

> _Add screenshots / GIFs here once published._

## 📦 Install

1. Grab the latest `Steam-Shadow-Launcher_<ver>_x64-setup.exe` from
   [Releases](../../releases).
2. Run it. The launcher will ask for admin rights only when actually needed
   (Sandbox mode requires elevation; Switch mode does not).
3. On first run, point it at your Steam install (auto-detected in 99% of
   cases) and pick a workspace folder for shadow-account data.

> **Sandboxie-Plus** is _not_ bundled — the launcher offers to download &
> silently install the latest official release from
> [sandboxie-plus/Sandboxie](https://github.com/sandboxie-plus/Sandboxie)
> the first time you switch to SANDBOX mode.

## 🚦 SWITCH vs SANDBOX — when to use what

|                       | SWITCH                              | SANDBOX                                              |
| --------------------- | ----------------------------------- | ---------------------------------------------------- |
| Closes main Steam     | Yes (graceful → -shutdown → kill)   | No                                                   |
| Simultaneous play     | ❌                                  | ✅                                                   |
| Re-download games     | ❌                                  | ❌ (host `steamapps` is mounted into the box)        |
| Needs admin           | No                                  | Yes (UAC prompt, with a heads-up dialog first)       |
| EAC / BattlEye        | ✅ Works                            | ⚠️  Some titles refuse to load inside a sandbox      |
| External dependencies | None                                | Sandboxie-Plus (silent install, ~12 MB)              |

## 🛡️ Safety

- `loginusers.vdf` is backed up under
  `<workspace>/backups/loginusers-<ts>.vdf` before every switch.
- Previous `HKCU\…\AutoLoginUser` is recorded in
  `<workspace>/backups/registry-<ts>.json`.
- Settings → **Revert last switch** restores both atomically.
- Steam files are never modified without a fresh backup first.
- The launcher does **not** store, ask for, or transmit Steam passwords.
  It only re-uses Steam's own auto-login token (the same mechanism Steam
  itself uses across reboots).

## 🛠️ Build from source

Prerequisites:
- Windows 10/11 x64
- [Rust stable](https://rustup.rs/) + MSVC build tools (Visual Studio 2022
  Build Tools with the "Desktop development with C++" workload)
- Node.js 20+
- (optional) [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
  — pre-installed on Windows 11

```powershell
git clone https://github.com/<you>/steam-shadow-launcher
cd steam-shadow-launcher
npm install
npm run tauri build
```

The resulting installer/portable exe lands in
`src-tauri/target/release/bundle/`.

## ❓ FAQ

**Will Steam ban me?**  No. The launcher only edits files Steam itself
edits: `loginusers.vdf` and `AutoLoginUser` in the registry. Both are
backed up and reversible. SAM / TcNo / Steam Account Manager have used the
same approach for over a decade.

**Steam Guard?**  The first time you launch a freshly imported account,
Steam Guard will prompt as usual. Subsequent launches re-use the saved
auto-login token, just like the official client does.

**Can I use it on Linux / macOS?**  No. Windows-only — the whole point is
Windows-specific reparse points (junctions), the Sandboxie driver, and the
Steam Win32 mutex namespace.

**Why is the window so small / can't I resize?**  Intentional. The whole
UI is laid out for a 760×520 pixel grid; resizing would just dilute the
typography. Open Settings to switch between RU / EN.

## 🙏 Credits

- [Sandboxie-Plus](https://sandboxie-plus.com/) by David Xanatos (GPLv3) —
  invoked as a separate process, never linked into our binary.
- [Tauri 2](https://tauri.app), [Vite](https://vitejs.dev),
  [Zustand](https://github.com/pmndrs/zustand).
- VT323 + JetBrains Mono fonts (open source).

## 📜 License

[MIT](LICENSE).

---

## 🇷🇺 Русский

> Маленький быстрый pixel-art лаунчер для Steam: переключает аккаунты в
> один клик или запускает **несколько Steam одновременно** через
> Sandboxie-Plus, не скачивая ни одной игры заново.

### ✨ Что умеет

- **SWITCH-режим** — гасит текущий Steam, патчит `loginusers.vdf` +
  `AutoLoginUser`, поднимает заново уже под выбранным аккаунтом. По духу
  — как SAM / TcNo Account Switcher, только с нормальным UI и бэкапами.
- **SANDBOX-режим** — запускает второй `steam.exe` в изолированном
  Sandboxie-Plus боксе. Оба Steam'а работают параллельно, библиотека
  `steamapps` шарится с основным (никаких повторных загрузок).
- **Выбор игры** — обложки берутся из локального `librarycache`,
  одним кликом стартует `steam://rungameid/<appid>` под нужным
  аккаунтом.
- **Ярлыки на рабочем столе** для каждого аккаунта (`.lnk` запускает
  лаунчер в headless-режиме и сразу стартует выбранный аккаунт).
- **Аватарки, избранное, счётчики запусков, ссылки на профиль/инвентарь.**
- **Chromeless pixel-art интерфейс** 760×520, RU/EN.
- **Не трогает оригинальные файлы Steam** без бэкапа.

### 📦 Установка

1. Скачай свежий `Steam-Shadow-Launcher_<ver>_x64-setup.exe` со страницы
   [Releases](../../releases).
2. Запусти. Админ-права запрашиваются только при необходимости (для
   SANDBOX-режима, со специальным предупреждением).
3. На первом запуске покажи путь к Steam (определяется автоматически в 99%
   случаев) и выбери папку для данных теневых аккаунтов.

> **Sandboxie-Plus** не входит в комплект — лаунчер предлагает скачать и
> установить последнюю официальную версию с
> [sandboxie-plus/Sandboxie](https://github.com/sandboxie-plus/Sandboxie)
> при первом включении SANDBOX-режима.

### 🚦 SWITCH vs SANDBOX

|                          | SWITCH                                           | SANDBOX                                                  |
| ------------------------ | ------------------------------------------------ | -------------------------------------------------------- |
| Закрывает основной Steam | Да (мягко → `-shutdown` → kill)                  | Нет                                                      |
| Параллельная игра        | ❌                                               | ✅                                                       |
| Перекачка игр            | ❌                                               | ❌ (хостовый `steamapps` пробрасывается в песочницу)     |
| Нужен админ              | Нет                                              | Да (UAC, с предупреждением до запроса)                   |
| EAC / BattlEye           | ✅                                               | ⚠️  Некоторые игры не запустятся внутри песочницы        |
| Сторонние зависимости    | Нет                                              | Sandboxie-Plus (silent install, ~12 МБ)                  |

### 🛡️ Безопасность

- `loginusers.vdf` бэкапится в `<workspace>/backups/` перед каждым
  переключением.
- Старое значение `HKCU\…\AutoLoginUser` сохраняется в
  `<workspace>/backups/registry-<ts>.json`.
- В Настройках есть кнопка **Revert last switch** — атомарно восстанавливает
  оба бэкапа.
- Лаунчер не хранит, не запрашивает и не передаёт пароли Steam: используется
  тот же auto-login токен, что и у самого клиента.

### 🛠️ Сборка из исходников

Нужны:
- Windows 10/11 x64
- Rust stable + MSVC build tools (VS 2022 Build Tools, Workload "Desktop
  development with C++")
- Node.js 20+

```powershell
git clone https://github.com/<you>/steam-shadow-launcher
cd steam-shadow-launcher
npm install
npm run tauri build
```

Артефакты — в `src-tauri/target/release/bundle/`.

### ❓ FAQ

**Меня забанят?**  Нет. Лаунчер правит только то, что и сам Steam правит
сотни раз в день: `loginusers.vdf` и `AutoLoginUser` в реестре. Оба
действия с бэкапом и обратимы. SAM / TcNo пользуются тем же приёмом
больше десяти лет.

**Steam Guard?**  При первом запуске только что импортированного аккаунта
Steam Guard сработает как обычно. Дальше используется сохранённый токен
авто-логина — точно так же, как у официального клиента.

**Linux / macOS?**  Нет, только Windows: используются junction reparse
points, драйвер Sandboxie и Win32 mutex'ы Steam.

**Почему окно фиксированного размера?**  Сознательное решение. Вся
типографика рассчитана на 760×520; resizable окно сломал бы
композицию.

### 🙏 Благодарности

- [Sandboxie-Plus](https://sandboxie-plus.com/) — David Xanatos (GPLv3),
  вызывается как внешний процесс, не линкуется в бинарник.
- [Tauri 2](https://tauri.app), [Vite](https://vitejs.dev),
  [Zustand](https://github.com/pmndrs/zustand).
- Шрифты VT323 и JetBrains Mono.

### 📜 Лицензия

[MIT](LICENSE).
