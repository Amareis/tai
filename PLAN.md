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

#### В процессе: Text→View, Write→Exec, уникальность title, кеш для exec

- [ ] `types.rs`: `BlockMode::Text` → `View`, `BlockMode::Write` → `Exec`, FromStr/Display
- [ ] `response/mod.rs`: убрать `heredoc_delim` из Header, убрать парсинг `<<DELIM` из `parse_block_header`, убрать `extract_heredoc_delimiter`, убрать `initial_heredoc` из `parse_block_body`, обновить тесты
- [ ] `core/mod.rs`: `TrackedCmd { title, command, rerun, cached_output, cached_exit }`, `upsert_tracked`, `apply_segments` (View/Exec/Close), `run_tracked` с кешем, убрать `execute_file_write`, удалить `utils.rs`
- [ ] `agent/llm.rs`: `Write` → `Exec`, `Text` → `View` в `prompt_to_messages`
- [ ] `prompt/system_prompt.txt`: переписать секции для view/exec
- [ ] `tests/server_test.rs`: `Text` → `View`, тесты exec + upsert
- [ ] cargo test + clippy

#### Следующие задачи

- [ ] Тест: полный lifecycle с реальным LLM
- [ ] Рассмотреть полезность `last_tick` (previous_response) в промпте
- [ ] Убрать `TestAgent` если не работает с новым Prompt
- [ ] Agent reasoning сохранять в debug

### Phase 5: Snapshot Tests

Цель: сервер тестируется с реальным бэкендом, мокается только Agent. Снепшоты = что видит модель на каждом шаге.

- [ ] Обновить TestAgent для работы с insta снепшотами
- [ ] Test harness: `tests/harness.rs`
- [ ] Snapshot tests (insta + toml)
- [ ] **Checkpoint**: `cargo test` зелёные, `cargo insta review` — читаемые TOML снепшоты

### Phase 6: Prompt Assembly (full)

Цель: слои, бюджет, layout trait, references.

- [ ] `prompt/layout.rs` — PromptLayout trait
- [ ] `prompt/budget.rs` — подсчёт токенов
- [ ] `prompt/references.rs` — сбор --help/man page
- [ ] Обновить снепшот тесты

### Phase 7: Session Persistence

Цель: перезапуск без потери данных.

- [ ] session.json read/write
- [ ] frozen content save/load
- [ ] Graceful shutdown + recovery
