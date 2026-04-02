# TAI — План реализации

> Фазы, задачи, зависимости. См. `ARCHITECTURE.md` для технических деталей.

---

## Требования к коду

**Строгие clippy-проверки.** Безопасный код по максимуму:

```toml
# Cargo.toml
[lints.clippy]
panic = "forbid"
indexing_slicing = "forbid"
unwrap_used = "forbid"
expect_used = "forbid"
```

- Никаких `.unwrap()`, `.expect()` — только `?` и явная обработка ошибок
- Никакой индексации `arr[i]` — только `.get(i)`, итераторы, pattern matching
- Никаких паник — код должен быть устойчивым к любым входным данным
- `thiserror` для всех error types

**Зависимости** — добавлять через `cargo add` без явных версий:

```bash
cargo add kitty-rc ratatui crossterm tokio clap serde serde_json ...
```

Не прописывать версии вручную в `Cargo.toml` — cargo сам подтянет актуальные. См. полный список в `ARCHITECTURE.md`.

---

## Phase 0: Фундамент

- [ ] Cargo workspace, все зависимости, clippy lints
- [ ] `types.rs` — Window, WindowState, Session, LaunchOpts, TaiCommand, TickTrigger
- [ ] `backend/mod.rs` — TerminalBackend trait
- [ ] `config.toml` + загрузка конфигурации
- [ ] Тесты сериализации типов
- [ ] Doc-комментарии для всех типов + перенести описание модели данных из `ARCHITECTURE.md`

## Phase 1: Kitty Backend — критический путь

- [ ] `backend/kitty.rs` — KittyBackend: launch (--hold), close, send-text, send-key, get-text, list-windows
- [ ] Kitty lifecycle: запуск (видимый по умолчанию, `--hidden` флаг), ожидание socket, heartbeat
- [ ] Process watch: poll `list_windows` каждые 500мс, detect `at_prompt`, post-mortem: get-text → Frozen → close
- [ ] Интеграционные тесты с реальным Kitty
- [ ] Doc-комментарии для TerminalBackend trait + KittyBackend + перенести из `ARCHITECTURE.md`

## Phase 2: Session Manager

- [ ] `manifest.rs` — session.json read/write
- [ ] `manager.rs` — Window lifecycle (Active→Frozen→Archived), focus/summarize, список окон для валидации
- [ ] `snapshot.rs` — frozen content save/load
- [ ] Doc-комментарии для session модуля + перенести из `ARCHITECTURE.md`

## Phase 3: Prompt Assembly

- [ ] `layout.rs` — PromptLayout trait + дефолтная реализация
- [ ] `budget.rs` — подсчёт токенов, бюджет слоёв (Immutable 20% / Ephemeral 70% / System 10%)
- [ ] `assembler.rs` — сборка через trait
- [ ] `references.rs` — сбор --help/man page для окон с правом записи
- [ ] Тесты лейаута
- [ ] Doc-комментарии для prompt модуля + перенести из `ARCHITECTURE.md`

## Phase 4: Routing + L-Model

- [ ] `parser.rs` — parse_blocks(): обязательные window:mode, валидация по списку окон
- [ ] `tai_command.rs` — парсер команд ядра (launch, close, focus, summarize)
- [ ] `l_model.rs` — HTTP клиент к Claude/GPT API, извлечение thinking
- [ ] Тесты парсера (валидные/невалидные блоки, tai:cmd)
- [ ] Doc-комментарии для routing модуля + перенести из `ARCHITECTURE.md`

## Phase 5: Kernel Event Loop

- [ ] `kernel/mod.rs` — main tick loop + trigger system (at_prompt / chat / idle timeout)
- [ ] Thinking → mind.md
- [ ] Обработка ошибок: изоляция между блоками, timeout (30с)
- [ ] Graceful shutdown (SIGINT/SIGTERM)
- [ ] Doc-комментарии для kernel модуля + перенести из `ARCHITECTURE.md`

## Phase 6: TUI

- [ ] `tui/mod.rs` — app state, ratatui setup, event loop
- [ ] `tui/chat.rs` — таб чата с моделью
- [ ] `tui/windows.rs` — список окон, focus/summarize/view frozen
- [ ] `tui/debug.rs` — пошаговое исполнение (Step / Run All / Edit / Skip)
- [ ] `tui/status_bar.rs` — status bar
- [ ] Doc-комментарии для tui модуля + перенести из `ARCHITECTURE.md`

## Phase 7: S-Models (отложено)

Лёгкие модели-наблюдатели для свёрнутых окон. Система полностью работает без них.

## Phase 8: IPC / Remote API (отложено)

Unix socket или HTTP API для внешних клиентов. Когда появится TmuxBackend или remote.
