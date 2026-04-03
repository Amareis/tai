# TAI — План реализации

> Фазы, задачи, зависимости. См. `ARCHITECTURE.md` для технических деталей.

---

## Phase 0: Фундамент ✅

- [x] Cargo workspace, все зависимости, clippy lints
- [x] `types.rs` — Window, WindowState, Session, LaunchOpts, TaiCommand, TickTrigger
- [x] `backend/mod.rs` — TerminalBackend trait
- [x] `config.toml` + загрузка конфигурации
- [x] Тесты сериализации типов (18 тестов)
- [x] Doc-комментарии для всех типов + перенести описание модели данных из `ARCHITECTURE.md`

## Phase 1: Kitty Backend ✅

- [x] `kitty-rc` crate в workspace (локальный, `crates/kitty_rc`)
- [x] `backend/kitty.rs` — KittyBackend: spawn (self-managed kitty), connect, launch (--hold), close, send-text, send-key, get-text, list-windows, set-title
- [x] Kitty lifecycle: `KittyBackend::spawn()` → spawn process → wait for socket → connect; Drop → kill child + remove socket
- [x] `backend/watch.rs` — ProcessWatch: poll `list_windows`, detect `at_prompt`, post-mortem: get-text → close → event
- [x] Юнит-тесты ProcessWatch с MockBackend (5 тестов) + 18 тестов Phase 0
- [ ] Интеграционные тесты с реальным Kitty (требует Kitty в CI/dev)
- [x] Doc-комментарии для KittyBackend + ProcessWatch

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
