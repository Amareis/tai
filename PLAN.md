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

Цель: `tai server` → Kitty открывается → оба интерфейа работают.

- [x] `client/mod.rs` — Unix Domain Socket server (в kernel) + client
    - Server: listen на сокете, accept, newline-delimited text protocol
    - Client: connect, stdin→socket (строки ввода), socket→stdout (строки вывода)
    - Client — ~50 строк, rustyline-async ↔ socket proxy (история команд, навигация по словам)
- [x] `client/model_view.rs` — текстовый вывод в socket
    - State header (окна, токены, focus) — просто строки текста
    - НЕ ratatui, НЕ ANSI форматирование — просто writeln в socket
    - Новые строки дописываются, никаких clear screen
- [x] Обновить `main.rs` — clap subcommands:
    - `tai server [--socket PATH] [--hidden]` — ratatui User Viewport на stdout,
      spawn Kitty с `kitty -- tai client --socket PATH`, ждёт socket connection
    - `tai client --socket PATH` — подключиться к сокету, stdin↔socket↔stdout, sleep
- [x] User Viewport: минимальный ratatui — "TAI Server (User Viewport)"
- [x] Model Channel: "TAI Model Workspace" через socket
- [x] **Checkpoint**: запускаю `tai server` → Kitty открывается → оба окна показывают текст → набираю текст в Kitty → server получает строку
- [ ] Doc-комментарии

### Phase 3: Command Parser + Window Operations ✅

Цель: печатаю `launch bash` в Kitty окно → появляется новое окно → вижу в User Viewport.

- [x] `backend/mod.rs` — `BackendCmd` enum + `CmdResponse` enum + отдельные структуры команд
    - LaunchCmd, SendTextCmd, SendKeysCmd, GetTextCmd, CloseCmd, SetTitleCmd
    - BackendCmd: Launch(LaunchCmd), SendText(SendTextCmd), SendKeys(SendKeysCmd), GetText(GetTextCmd), Close(CloseCmd), List, SetTitle(SetTitleCmd)
    - CmdResponse: WindowCreated(WindowId), Text(String), Windows(Vec<WindowInfo>), Ok, Error(String)
- [x] Обновить `TerminalBackend` trait: один метод `async fn execute(&self, cmd: BackendCmd) -> Result<CmdResponse>`
- [x] Реализовать `execute()` в `KittyBackend` — dispatch по вариантам BackendCmd
- [x] `routing/parser.rs` — clap-based парсер текстовых команд из Model Channel
    - Reuse clap (уже в зависимостях для `tai server`/`tai client`)
    - Subcommands: `launch [--title NAME] -- <cmd>`, `send <window> <text...>`, `keys <window> <keys...>`, `get <window>`, `close <window>`, `list`, `title <window> <title>`
    - `clap::try_parse_from()` → `BackendCmd` или сразу error feedback
    - Валидация: launch требует непустую команду
- [x] Чтение ввода из Model Channel (socket lines → clap parser → dispatch)
- [x] Dispatch: BackendCmd → backend.execute() (единый метод trait'а)
- [x] Model Channel: результат команды (текстовый feedback: "Window created: 123")
- [x] Тесты парсера (16 тестов: валидные/невалидные команды, clap error messages)
- [x] Тесты ProcessWatch обновлены для нового API
- [x] **Checkpoint**: в Kitty окне набираю `launch bash` → появляется Kitty tab → `list` → вижу оба окна → `close 1` → окно закрылось
- [x] Doc-комментарии
- [x] Debug mode (`--debug`): TUI не рисуется, только логи

User Viewport отложен до Phase 8 — окна видны напрямую в терминале.

### Phase 4: Agent Trait + Snapshot Tests

Цель: сервер тестируется напрямую с реальным Kitty backend, мокается только модель. Снепшоты = что видит модель.

Не мокаем терминальный backend — он реальный (Kitty). Мокаем только Agent (LLM).
MockAgent сохраняет полученный промпт в insta снепшот, возвращает захардкоженный ответ,
снова получает промпт — снова снепшот — столько раз сколько нужно сценарию.
Снепшоты в TOML — одновременно тесты и документация "что видит модель".

- [ ] `models/mod.rs` — trait `Agent` (интерфейс взаимодействия с моделью)
    - `async fn step(&self, prompt: &str) -> Result<String>` — получает собранный промпт, возвращает ответ
    - В проде: вызывает LLM API (реализация в Phase 7)
    - В тестах: MockAgent
- [ ] `MockAgent` — пошаговый сценарий для тестов
    - Конфигурируется последовательностью шагов: `MockAgent::new().step("ответ1").step("ответ2")`
    - Каждый вызов: 1) сохраняет полученный промпт в insta снепшот 2) возвращает захардкоженный ответ
    - `mock.assert_all_steps_consumed()` — проверка что сценарий пройден полностью
- [ ] Test harness: `tests/harness.rs`
    - Создаёт сервер напрямую (не subprocess) с реальным KittyBackend + MockAgent
    - Helpers: `harness.input("launch bash")` → сервер обрабатывает → ответ в Model Channel
    - Helpers: `harness.wait_tick()` — подождать полный tick cycle
    - Helpers: `harness.freeze(id, exit_code)` — дождаться at_prompt → freeze → проверить
    - Окна независимы → тесты параллельно без мьютексов
- [ ] Snapshot tests (insta + toml) — что модель видит на каждом шаге:
    - Пустой промпт (только system + mind)
    - Одно active окно → dashboard + focused content
    - Frozen окно → exit code + content в наблюдениях
    - Несколько окон → dashboard показывает все, focused — только выбранные
    - Предыдущий ответ модели → стоит перед текущими наблюдениями
    - Full tick cycle: input → mock видит промпт → снепшот → возвращает ответ → parse → execute → mock видит следующий промпт → снепшот
- [ ] Парсинг ответа модели (code blocks) — тоже через снепшоты:
    - ````build:text\ncargo build\n````` → `ParsedSegment::Block{window: "build", mode: Text, content: "cargo build"}`
    - Prose текст → `ParsedSegment::Prose`
    - Невалидные блоки → `ParsedSegment::Invalid`
- [ ] **Checkpoint**: `cargo test` зелёные, `cargo insta review` — снепшоты читаемые TOML, показывают всю структуру промпта
- [ ] Doc-комментарии

### Phase 5: Window Lifecycle

Цель: команда завершилась → ядро обнаружило → захватило вывод → окно frozen с exit code.

- [ ] `session/manager.rs` — Window lifecycle (Active → Frozen → Archived)
    - Window struct: id, title, pid, state, content (Option<String>), exit_code (Option<i32>)
    - Session struct: Vec<Window>, tracked windows, focus state
    - Active → Frozen: захватить get-text → сохранить content + exit code → close окно
- [ ] `backend/watch.rs` — интеграция at_prompt в event loop
    - Poll каждые 500мс для tracked окон
    - at_prompt == true → emit event (WindowDone(window_id))
    - Event → trigger freeze flow
- [ ] Focus/summarize: управление какие окна в "контексте"
    - `focus <window>` — окно развёрнуто, его content попадёт в промпт
    - `unfocus <window>` — окно свёрнуто (summary вместо полного content)
- [ ] Команды lifecycle через Model Channel:
    - `list` — список с состояниями (Active/Frozen/Archived), exit code для frozen
    - `focus <id>` / `unfocus <id>`
    - `archive <id>` — frozen → archived (не в контексте)
- [ ] Frozen content: хранить в RAM (Vec<String>), без записи на диск (persistence — отдельная фаза)
- [ ] **Checkpoint**: `launch -- bash -c "echo hello && sleep 1"` → ждём → `windows` показывает frozen с exit code 0 + content "hello" → `focus 1` → content доступен
- [ ] Тесты lifecycle с MockBackend
- [ ] Doc-комментарии

### Phase 6: Prompt Assembly

Цель: вижу собранный промпт в debug output. Реальные данные из окон.

- [ ] `prompt/layout.rs` — PromptLayout trait + дефолтная реализация
    - Immutable layer: system prompt + mind.md
    - Ephemeral layer: dashboard + focused windows + previous response
    - System layer: status bar (tokens, windows, write target)
- [ ] `prompt/budget.rs` — подсчёт токенов (tiktoken-rs), бюджет слоёв
- [ ] `prompt/assembler.rs` — сборка промпта через PromptLayout
- [ ] `prompt/references.rs` — сбор --help/man page для окон с правом записи
- [ ] Model Channel: команда `prompt` → показывает собранный промпт (для debug)
- [ ] **Checkpoint**: открываю несколько окон → `prompt` → вижу полный промпт с содержимым окон, mind, budget
- [ ] Тесты layout (mock данные, проверка структуры и бюджета)
- [ ] Doc-комментарии

### Phase 7: L-Model + Event Loop

Цель: полный цикл. Пишу в Kitty → модель отвечает → ядро исполняет → результат виден.

- [ ] `models/l_model.rs` — LLM клиент через llm crate (Claude/GPT API)
- [ ] Расширить parser: модель отвечает markdown с code blocks (```window:mode```)
    - Человек: plain commands через socket (из Phase 3)
    - Модель: code blocks с обязательными window:mode
    - Один парсер, два input format
- [ ] `kernel/mod.rs` — main tick loop
    - Триггеры: at_prompt (command done) / user input from Model Channel socket / idle timeout
    - Tick: assemble → invoke model → parse response → execute blocks → wait
    - Blocks выполняются параллельно
- [ ] Thinking → mind.md (извлечение из extended thinking)
- [ ] Обработка ошибок: изоляция между блоками, timeout (30с)
- [ ] Model Channel: показывает ответы модели (prose + executed blocks) как plain text
- [ ] User Viewport: debug view — что модель решила, что выполнилось
- [ ] **Checkpoint**: пишу в Kitty "найди все TODO в проекте" → модель открывает окно с grep → результат виден → модель докладывает
- [ ] Doc-комментарии

### Phase 8: TUI Polish

Цель: полноценный интерактивный dashboard в User Viewport.

- [ ] Model Channel polish:
    - Подсветка code blocks ANSI цветами
    - Красивый state header
    - Команда `clear` для очистки лога (scrollback сохраняется)
- [ ] User Viewport — Debug UI:
    - Windows tab: список с фильтрами, preview frozen content
    - Debug tab: пошаговое исполнение (Step / Run All / Edit / Skip)
    - Status bar: токены, активные окна, write target, tick count
- [ ] `tui/status_bar.rs` — общие компоненты
- [ ] Обработка горячих клавиш в User Viewport
- [ ] **Checkpoint**: полноценная интерактивная сессия — Model Channel как REPL, User Viewport как dashboard

### Phase 9: Session Persistence

Цель: перезапускаю `tai server` → frozen данные на месте, история сохранена.

- [ ] `session/manifest.rs` — session.json read/write (window registry, metadata)
- [ ] `session/snapshot.rs` — frozen content save/load на диск
- [ ] Graceful shutdown: freeze all active → save manifest → kill Kitty
- [ ] Recovery при старте: load manifest → reconnect/recreate windows
- [ ] **Checkpoint**: working session с несколькими frozen окнами → Ctrl+C → `tai server` → frozen данные доступны
- [ ] Тесты persistence (save/load round-trip)
- [ ] Doc-комментарии

## Открытые вопросы

### `keys` command bug

`keys` отправляет символы как текст вместо keypress events. Возможно проблема в kitty-rc протоколе или формате `send-keys`. Исследовать позже.

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

## S-Models

Лёгкие модели-наблюдатели для свёрнутых окон. Система полностью работает без них.

## IPC / Remote API 

Unix socket или HTTP API для внешних клиентов. Когда появится TmuxBackend или remote.
