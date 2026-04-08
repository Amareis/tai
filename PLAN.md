# TAI — План реализации

> Фазы, задачи, зависимости. См. `ARCHITECTURE.md` для технических деталей.

---

> Принцип: каждая фаза = runnable checkpoint. После каждого шага можно запустить
> бинарник и проверить руками (и автотестами) что работает.

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

### Phase 2: Unix Socket + Minimal Dual Interface ✅

Цель: `tai server` → Kitty открывается → оба интерфейса работают.

- [x] `client/mod.rs` — Unix Domain Socket server (в kernel) + client
- [x] `client/model_view.rs` — текстовый вывод в socket
- [x] Обновить `main.rs` — clap subcommands
- [x] User Viewport: минимальный ratatui — "TAI Server (User Viewport)"
- [x] Model Channel: "TAI Model Workspace" через socket
- [x] **Checkpoint**: запускаю `tai server` → Kitty открывается → оба окна показывают текст

### Phase 3: Command Parser + Window Operations ✅

- [x] `backend/mod.rs` — `BackendCmd` + `CmdResponse` + структуры команд
- [x] `TerminalBackend` trait: `async fn execute(&self, cmd: BackendCmd) -> Result<CmdResponse>`
- [x] `KittyBackend::execute()` — dispatch по BackendCmd
- [x] `routing/parser.rs` — clap-based парсер текстовых команд
- [x] Чтение ввода из Model Channel → clap parser → dispatch
- [x] Тесты парсера (16 тестов)
- [x] **Checkpoint**: в Kitty окне набираю `launch bash` → появляется окно → `list` → `close`

### Phase 4: MVP — Tick Cycle (в процессе)

Цель: первый полный цикл. Модель получает структурированный промпт, возвращает структурированный ответ, ядро исполняет, сессия отслеживает окна.

#### Готово:

- [x] `models/mod.rs` — trait `Agent`
    - `async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError>`
    - Принимает структурированный `Prompt`, возвращает структурированный `AgentResponse`
- [x] `prompt/mod.rs` — `Prompt` struct
    - `system`, `dashboard` (Vec<WindowSummary>), `focused_windows` (Vec<WindowView>), `previous_response`, `status` (StatusInfo)
    - `Prompt::to_text()` для рендеринга в текст (реальные LLM-агенты)
- [x] `models/mod.rs` — `AgentResponse` struct
    - `segments: Vec<ParsedSegment>` + builder-методы: `empty()`, `prose()`, `block()`, `and()`
- [x] `models/mod.rs` — `NopAgent` (пустой ответ), `MockAgent` (очередь ответов), `TestAgent` (пошаговая проверка)
    - TestAgent: `.step(check_fn, response).step(...)` + `assert_all_consumed()`
- [x] `response/mod.rs` — парсинг code blocks из текста в `ParsedSegment`
- [x] `Server` в `core/mod.rs` — tick, execute_blocks, execute_block
- [x] Server владеет `Box<dyn Agent>`, `Session`, `Box<dyn TerminalBackend>`
- [x] `Server::accept(back, session, agent, listener)` — всё приходит снаружи
- [x] E2E тест: `tick_empty_session` — пустая сессия, TestAgent проверяет структуру промпта ✅

#### Осталось (TDD — тест уже написан, падает):

Единый принцип: **Server никогда не мутирует session напрямую**. Все изменения окон — через watcher.

Флоу:
1. Agent → `launch --title foo -- make test` → `execute_block` → backend возвращает `WindowCreated(id)`
2. `execute_block` → `watcher.track(id, title)` — только регистрирует, session не трогает
3. Watcher (background task) владеет `Arc<Mutex<Session>>` + backend:
   - Поллит kitty (`List` + `Get`) периодически
   - При первом poll нового id: создаёт `Window::new_active`, добавляет в session, focused
   - При exit (detected through `last_cmd_exit_status` в get-text): читает финальный контент → `Active → Frozen` в session → шлёт `WindowExited` сигнал
4. Watcher сигнализирует через `tokio::sync::mpsc`:
   - `WatchEvent::WindowExited(window_id, exit_code)` — основной триггер
   - `WatchEvent::WindowOutput(window_id)` — опционально для MVP
5. Server event loop:
   ```rust
   loop {
       select! {
           event = watcher_rx.recv() => { tick(event).await }
           line = client.read_line() => { handle_user_input(line) }
       }
   }
   ```
   TickTrigger: `WindowExited(window_id, exit_code)`, `UserMessage(text)`

- [ ] **Watcher::track(id, title)** — регистрация нового окна для отслеживания
- [ ] **Watcher background task** — владеет session + backend, поллит, мутит session
    - Добавляет Window в session при первом poll
    - Freeze при exit: `last_cmd_exit_status` из get-text → `Active → Frozen { content, exit_code }`
    - Шлёт `WatchEvent` через channel
- [ ] **Server event loop** — `select!` на watcher events + client input
    - `execute_block` при launch → только `watcher.track()`, не трогает session
- [ ] **Обёртка команд в bash -c**: kitty корректно показывает `last_cmd_exit_status` только если процесс — bash
- [ ] **Тест: launch → session tracks** (уже написан, красный)
- [ ] **Тест: полный lifecycle**: launch → watcher detects exit → freeze → tick показывает frozen окно
- [ ] Doc-комментарии

### Phase 5: Snapshot Tests

Цель: сервер тестируется с реальным Kitty, мокается только Agent. Снепшоты = что видит модель на каждом шаге.

- [ ] Обновить TestAgent для работы с insta снепшотами
- [ ] Test harness: `tests/harness.rs`
- [ ] Snapshot tests (insta + toml)
- [ ] **Checkpoint**: `cargo test` зелёные, `cargo insta review` — читаемые TOML снепшоты

### Phase 6: Window Lifecycle (full)

Цель: полноценное управление окнами — focus, unfocus, archive.

- [ ] Focus/unfocus: управление какие окна в "контексте"
- [ ] Archive: frozen → archived (не в контексте, не в RAM)
- [ ] `windows` команда — список с состояниями
- [ ] Тесты

### Phase 7: Prompt Assembly (full)

Цель: слои, бюджет, layout trait, references.

- [ ] `prompt/layout.rs` — PromptLayout trait
- [ ] `prompt/budget.rs` — подсчёт токенов
- [ ] `prompt/references.rs` — сбор --help/man page
- [ ] Обновить снепшот тесты

### Phase 8: Real LLM Client

Цель: подключаем реальную модель.

- [ ] `models/l_model.rs` — LLM клиент через llm crate (Claude/GPT API)
- [ ] Thinking → mind.md
- [ ] Обработка ошибок, timeout

### Phase 9: TUI Polish

Цель: полноценный интерактивный dashboard.

- [ ] Model Channel polish (ANSI, state header)
- [ ] User Viewport — Debug UI (windows tab, debug tab, status bar)

### Phase 10: Session Persistence

Цель: перезапуск без потери данных.

- [ ] session.json read/write
- [ ] frozen content save/load
- [ ] Graceful shutdown + recovery

## Открытые вопросы

### `keys` command bug

`keys` отправляет символы как текст вместо keypress events. Исследовать позже.

### Stderr routing

Три stderr-потока: kernel, kitty child, window process. См. `ARCHITECTURE.md`.

## S-Models

Лёгкие модели-наблюдатели для свёрнутых окон. Система полностью работает без них.

## IPC / Remote API

Unix socket или HTTP API для внешних клиентов. Когда появится TmuxBackend или remote.
