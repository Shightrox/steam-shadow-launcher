**Как читать воспроизведения**

[reproductions.rs](reproductions.rs) — восемь проверок, выполненных при ревью. Они подтверждают ошибочное текущее поведение: положительный результат означает, что дефект воспроизведён. Файл намеренно не подключён к обычной сборке и не заменяет регрессионные тесты исправлений.

| Тест | Пункт отчёта |
| --- | --- |
| `review_dot_login_escapes_accounts_directory` | 09 |
| `review_duplicate_add_resets_authenticator_and_favorite` | 12 |
| `review_move_merges_and_overwrites_existing_destination_account` | 01 |
| `review_locked_vault_writes_new_account_in_plaintext` | 02 |
| `review_partial_rekey_leaves_first_account_unreadable_with_cached_key` | 04 |
| `review_reqwest_display_contains_token_query` | 10 |
| `review_failed_master_password_change_changes_cached_key` | 03 |
| `review_import_accepts_another_steam_account` | 05 |

Для повторного запуска на Windows временно добавить в `src-tauri/src/main.rs`:

```rust
#[cfg(test)]
#[path = "../../docs/review-2026-09-19/reproductions.rs"]
mod review_regressions;
```

Затем из корня репозитория:

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml review_regressions -- --test-threads=1
```

После проверки убрать временное подключение. Запускать именно с фильтром и одним потоком: часть тестов меняет процессный кеш ключа и временно перенаправляет `APPDATA` внутри тестового процесса. Все аккаунты, пароли и maFile искусственные; файловые изменения выполняются в уникальных временных каталогах. Запрос с фиктивным токеном направлен на loopback `127.0.0.1:1`, а не в Steam.

Во время ревью результат: **8 passed, 0 failed**. Исходное приложение после проверки восстановлено без подключения этого модуля. При исправлении каждого дефекта перенести проверку в соответствующий модуль и заменить утверждения на ожидаемое безопасное поведение.
