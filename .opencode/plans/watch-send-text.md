# План: Все окна — живые шеллы, send-text для команд

## Суть

Каждое окно — живой `zsh` на `--hold`. Команда отправляется через `send-text`. Каждый тик все трекнутые окна ре-запускаются: `clear && {cmd}\n` → wait `at_prompt` → `get-text`. ID не меняется, окна не закрываются, не пересоздаются.

Никакого one-shot. Нет `zsh -c`. Нет close/re-launch.

## Концепция

**Только один режим**: живой шелл. Launch пустой `zsh` с `--hold` + title, команду отправляем через `send-text`. Каждый тик — `clear && {cmd}\n` в то же окно.

## Изменения

### 1. `backend/watch.rs` — упростить TrackedWindow

Убрать `was_active` (не нужно больше, polling exit не используется). Убрать `is_watch` (все окна watch).

```rust
struct TrackedWindow {
    id: WindowId,
    title: String,
    command: String,
}
```

`track(id, title, command)` — 3 аргумента.

`watch_entries()` — возвращает все (все watch).

Убрать `update_id` (ID не меняется).

Оставить `remove_by_title`.

Убрать `poll_exited` (не нужен, триггер — idle timeout).

### 2. `core/mod.rs` — `execute_text` (бывший `execute_one_shot`)

Вместо `launch zsh -c "cmd"`:

1. `launch zsh` (пустой шелл, `--hold`, title)
2. `send-text` `"{cmd}\n"`
3. `wait_for_window(id, timeout)` — ждём `at_prompt`
4. `track(id, title, cmd)`

Новый `LaunchCmd`: `command` пустой строки → запускать просто `zsh`.

### 3. `core/mod.rs` — `rerun_watch_windows` (бывший `refresh_watch_windows`)

```
for each tracked window:
    send_text(id, "clear && {command}\n")
    wait_for_window(id, timeout)  // wait for at_prompt
```

Никаких close, никаких re-launch. ID не меняется.

### 4. `core/mod.rs` — начальные окна в `run()`

Тот же путь что `execute_text`: launch → send-text → track.

```rust
let id = back.cmd_launch(LaunchCmd { title: Some("task"), command: String::new() }).await?;
back.execute(BackendCmd::Send(SendTextCmd { window: id.clone(), text: vec!["cat TASK.md".into()] })).await?;
watcher.track(id, "task", "cat TASK.md");
// wait for at_prompt...
```

### 5. `core/mod.rs` — убрать

- `update_id` — не нужен
- `poll_exited` использование — убрать из `wait_trigger`
- `refresh_watch_windows` — заменить на `rerun_watch_windows`

### 6. `backend/kitty.rs` — `cmd_launch` пустой command

Если `command.is_empty()` — запускать просто `zsh` без `-c`.

### 7. Тесты

- Обновить `track()` вызовы (3 аргумента, без `is_watch`)
- Убрать `test_update_id`
- Обновить `kitty_test.rs`
- `watch_entries()` возвращает все entries (нет фильтра)

### 8. Tick loop

`tick()`:
1. `rerun_watch_windows()` — send `clear && cmd` во все трекнутые окна, wait at_prompt
2. Собрать промпт (get-text всех окон)
3. Вызвать агента
4. Исполнить блоки (execute_text для новых, execute_close для :close, execute_file_write для :write)

## Файлы

| Файл | Изменение |
|------|-----------|
| `src/backend/watch.rs` | Упростить TrackedWindow (убрать was_active, is_watch, update_id), track(id,title,cmd), watch_entries() без фильтра |
| `src/core/mod.rs` | `execute_text` (launch zsh + send-text), `rerun_watch_windows` (send-text, не close/re-launch), `run()` через launch+send-text, убрать update_id, poll_exited из wait_trigger |
| `src/backend/kitty.rs` | Пустой command → запуск `zsh` без `-c` |
| `src/backend/mod.rs` | Убрать `was_active` / `is_at_prompt` из Terminal если poll_exited убран (или оставить для wait_for_window) |
| `tests/kitty_test.rs` | Обновить track() вызовы |