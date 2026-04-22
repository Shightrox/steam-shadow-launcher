# Changelog

## v0.2.1 — Auto-updater

### Added

- **Проверка обновлений при старте.** Через ~4 с после запуска лаунчер тихо спрашивает GitHub Releases о последней версии. Если доступна новее — показывается модалка с release-notes и кнопками «Обновить» / «Позже».
- **Обновление в один клик.** По «Обновить» свежий `*-portable.exe` скачивается рядом с текущим, старый бинарь переименовывается в `*.old`, новый занимает его место, затем детач-скрипт (`%TEMP%\ssl-update-*.cmd`) ждёт завершения процесса, чистит `.old` и заново запускает лаунчер. Юзеру не надо ничего перетаскивать.
- Ссылка «На GitHub» в модалке — открывает страницу релиза.

### Notes

- Обновление работает только для портативной сборки (скачивает asset c именем, содержащим `portable` и заканчивающимся на `.exe`).
- Проверка best-effort: офлайн / rate-limit / отсутствие asset'а не ломают загрузку лаунчера.
- Никаких скрытых автообновлений без явного подтверждения.

## v0.2.0 — Steam Desktop Authenticator

Лаунчер теперь умеет работать со Steam Guard сам, без мобильного приложения.

### Added

- **Steam Guard TOTP** рядом с каждой карточкой аккаунта, который привязан к лаунчеру.
- **Импорт `.maFile`** из SDA / steamguard-cli. Поддерживается и plain, и encrypted (PBKDF2 + AES-CBC, SDA-совместимый формат).
- **Экспорт `.maFile`** обратно.
- **Мастер привязки нового Steam Guard** (13 фаз): login → diagnose (QueryStatus + PhoneStatus) → phone / email-Guard путь → activation → revocation-code → persist. Работает и для аккаунтов без привязанного телефона (email-Guard, `validate_sms_code=1` с активационным кодом из письма).
- **Список подтверждений** (trades / market / phone / login) с allow/reject и bulk-операциями.
- **Фоновый поллер подтверждений** с настраиваемым интервалом. Умеет авто-подтверждать только исходящие трейды (`creator_id == our steam_id`) и любые market-листинги.
- **Мастер-пароль** для всех `.maFile` в workspace — Argon2id (64 MiB, t=3, p=1) + AES-256-GCM. Ключ живёт только в памяти до `lock`.
- **Отвязка аутентификатора** через revocation-код прямо из UI.
- Вкладки `Accounts / Authenticator` в сайдбаре.
- RU/EN локализация всех новых строк.

### Changed

- Сайдбар перерисован: табы теперь стекаются вертикально с левой полосой-индикатором активной вкладки (pixel-шрифт не умещал два таба в grid 1fr/1fr).
- Destructive-confirm диалоги (remove account, remove authenticator, disable master password) — type-to-confirm с 600 мс grace и авто-фокусом на Cancel.
- Confirm-диалоги перекрывают обычные модалки (z-index 210 через `:has()`).

### Fixed

- `BeginAuthSessionViaCredentials/v1` теперь отправляет `input_json` вместо `input_protobuf_encoded` (Steam отклонял protobuf для web-login).
- `submit_code` корректно читает `x-eresult`-заголовок (Steam отвечает HTTP 200 даже на неверный Guard-код, настоящий статус только в header).
- Константы `EAuthSessionGuardType` приведены в соответствие со Steam (EmailCode=2, DeviceCode=3, DeviceConfirmation=4, EmailConfirmation=5, MachineToken=6).
- Email-Guard детектится через `allowed_confirmations` из login-шага (`QueryStatus` не различает «нет Guard» и «email Guard on» — оба возвращают `state=0`).
- `access_token` автоматически обновляется через `refresh_token` при HTTP 401 во всех authed-запросах модуля add (раньше recovery был только в `diagnose`).
- **Защита от потери ключей**: после `AddAuthenticator/v1` партиальный `.maFile` с `fully_enrolled=false` пишется на диск немедленно, до показа revocation-UI. Если окно закроется — секреты не потеряются.
- `FinalizeAddAuthenticator/v1` для email-Guard аккаунтов теперь требует `validate_sms_code=1` с кодом из письма (пробовать «без SMS» бесполезно, Steam всё равно отвергает).
- Поллер: per-login экспоненциальный backoff (15 → 300 с), полу-привязанные аккаунты (`fully_enrolled=false`) пропускаются.
- `diagnose` троттлится на 2 с — UI дёргает её по несколько раз подряд на каждый phase change.
- `LoginFlowModal`: поле Guard-кода сбрасывается при возврате в фазу `code` (Steam отверг старый код).
- TitleBar close prompts при активном wizard-е.
- Warning-checkbox при включении master-password («восстановить нельзя»).
- Status-body в логах `diagnose` обрезается до 200 символов.

### Security

- `shared_secret`, `identity_secret`, `refresh_token` не попадают в логи ни на одном этапе.
- `revocation_code` показывается в UI ровно один раз, после чего живёт только в `.maFile` на диске (в шифрованном виде, если мастер-пароль включён).
- Все секреты держатся в Rust-процессе и никогда не отправляются на JS-слой в plaintext, кроме revocation-кода на этапе «покажи юзеру один раз».

### Notes

- Clean-room реализация протокола Steam Mobile Authenticator. Код SDA и steamguard-cli (оба GPL-3.0) не копировался — использовались только публичные описания endpoint'ов.

---

## v0.1.0 — Initial release

- SWITCH mode — account switching via `loginusers.vdf` + registry.
- SANDBOX mode — parallel Steam clients via Sandboxie-Plus.
- Game picker, per-account shortcuts, favorites, launch counters.
- Backups & Revert last switch.
- Portable + NSIS installer builds.
