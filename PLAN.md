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
- [x] Интеграционные тесты с реальным Kitty (требует Kitty в CI/dev)
- [x] Doc-комментарии для KittyBackend + ProcessWatch

## Phase 2: FD Passing + Dual Viewport

- [ ] Добавить зависимости: `nix` (SCM_RIGHTS), `passfd` или аналог
- [ ] `fd/mod.rs` — Unix Domain Socket server/client
  - Server: listen на `/tmp/tai.sock`, accept, recv FD
  - Client: connect, send FD (stdin + stdout), sleep
- [ ] `fd/terminal_manager.rs` — TerminalManager: `HashMap<ViewId, Terminal<CrosstermBackend<File>>>`
  - Создание terminal на FD
  - `draw_all()` — перерисовка всех viewport'ов
- [ ] Обновить `main.rs` — clap subcommands:
  - `tai server [--socket PATH] [--hidden]` — spawn Kitty с `tai client`, ждать FD
  - `tai client --socket PATH` — подключиться, передать FD, sleep
- [ ] SIGWINCH в client → resize message → server вызывает `terminal.resize()`
- [ ] Адаптация e2e тестов: `tai server` → Kitty открывается → оба viewport рисуют
- [ ] Doc-комментарии для fd модуля

## Phase 3: Session Manager

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

## Phase 7: Dual TUI Viewports

- [ ] `tui/mod.rs` — TerminalManager интеграция, event loop для обоих viewport
- [ ] `tui/model_view.rs` — Model Viewport (Kitty окно):
  - Chat: диалог с моделью, человек пишет напрямую
  - Context view: focused окна, dashboard, previous response
  - Status bar
- [ ] `tui/user_view.rs` — User Viewport (терминал человека):
  - Windows tab: список окон, focus/summarize/view frozen
  - Debug tab: пошаговое исполнение (Step / Run All / Edit / Skip)
  - Status bar
- [ ] `tui/status_bar.rs` — общие компоненты
- [ ] Doc-комментарии для tui модуля + перенести из `ARCHITECTURE.md`

## Открытые вопросы

### Stderr routing (три канала)

Три разных stderr-потока в системе, каждый требует своего решения:

1. **TAI kernel stderr** — сейчас `eprintln!`. Сломает TUI когда ratatui захватит терминал.
   → Решение: `tracing` с записью в log file (`~/.local/share/tai/kernel.log`).
   Debug tab может показывать tail этого файла.

2. **Kitty child process stderr** — сейчас `Stdio::null()` в `kitty.rs:55`. Теряем диагностику Kitty
   (ошибки RC protocol, предупреждения).
   → Решение: pipe stderr Kitty → async buffer → доступно модели как feedback.
   Возможно стоит выводить в отдельное TUI окно или feed в debug tab.

3. **Window process stderr** — через PTY, смешан с stdout. Модель видит через get-text. ✅ Не требует изменений.

**Открытый вопрос:** как именно stderr Kitty процесса интегрируется в tick cycle — отдельное окно?
Строка в dashboard? Event в kernel loop? Решить при реализации Phase 5/6.

## Phase 7: S-Models (отложено)

Лёгкие модели-наблюдатели для свёрнутых окон. Система полностью работает без них.

## Phase 8: IPC / Remote API (отложено)

Unix socket или HTTP API для внешних клиентов. Когда появится TmuxBackend или remote.
