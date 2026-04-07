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

User Viewport отложен до Phase 9 — окна видны напрямую в терминале.

### Phase 4: MVP — Minimal Tick Cycle

Цель: первый полный тик. Пишу в Kitty → (mock) модель отвечает → ядро парсит → исполняет → результат виден.

Вытаскиваем минимум из будущих Phase 5-7 — ровно столько чтобы тик работал.
Всё упрощённое: плоский промпт, базовый lifecycle, без бюджета/слоёв.

- [ ] `models/mod.rs` — trait `Agent`
    - `async fn step(&self, prompt: &str) -> Result<String>` — получает промпт, возвращает ответ
    - Пока без реальной реализации — Agent trait + минимальный stub
- [ ] Prompt v1 (плоский, без слоёв/budget):
    - system prompt (захардкоженный) + dashboard (список окон) + focused content (get-text)
    - Предыдущий ответ модели перед текущими наблюдениями
    - Собирается одной функцией, без trait abstraction
- [ ] Парсинг ответа модели (code blocks):
    - Regex для ` ```window:mode\ncontent\n``` ` → `ParsedSegment::Block / Prose / Invalid`
    - Режимы: text, keys, cmd (tai:cmd)
- [ ] Window lifecycle v1 (минимальный):
    - Active → Frozen: at_prompt → get-text → content + exit code → close
    - Frozen content в RAM, без диска
    - Триггер: at_prompt (process done) → freeze → следующий тик
- [ ] Tick loop (`kernel/mod.rs`):
    - assemble prompt → agent.step() → parse response → execute blocks → wait
    - Триггеры: user input (socket) + at_prompt (process done)
    - Blocks выполняются параллельно
    - Execute: text → send-text, keys → send-key, tai:cmd → dispatch
- [ ] Model Channel: prose + executed blocks → plain text в socket
- [ ] **Checkpoint**: `tai server` → пишу "запусти билд" → (mock) модель возвращает `\`\`\`build:text\ncargo build\n\`\`\`` → окно создаётся → команда выполняется → окно freezes → следующий тик показывает результат
- [ ] Doc-комментарии

### Phase 5: Snapshot Tests

Цель: сервер тестируется с реальным Kitty, мокается только Agent. Снепшоты = что видит модель на каждом шаге.

Опирается на MVP tick cycle из Phase 4. MockAgent подменяет Agent trait.
Каждый вызов: снепшот промпта → вернуть захардкоженный ответ → repeat.

- [ ] `MockAgent` — пошаговый сценарий
    - `MockAgent::new().step("ответ1").step("ответ2")`
    - Каждый вызов `step()`: 1) insta снепшот полученного промпта 2) возвращает захардкоженный ответ
    - `mock.assert_all_steps_consumed()`
- [ ] Test harness: `tests/harness.rs`
    - Сервер напрямую (не subprocess) с реальным KittyBackend + MockAgent
    - `harness.input("launch bash")` → сервер обрабатывает
    - `harness.wait_tick()` — подождать полный tick cycle
    - `harness.freeze(id, exit_code)` — дождаться at_prompt → freeze → проверить
    - Окна независимы → тесты параллельно
- [ ] Snapshot tests (insta + toml):
    - Пустой промпт (только system)
    - Active окно → dashboard + content
    - Frozen окно → exit code + content
    - Несколько окон, focus/unfocus
    - Предыдущий ответ модели перед наблюдениями
    - Full tick cycle сценарии: multi-step
- [ ] Парсинг ответа — снепшоты ParsedSegment
- [ ] **Checkpoint**: `cargo test` зелёные, `cargo insta review` — читаемые TOML снепшоты

### Phase 6: Window Lifecycle (full)

Цель: полноценное управление окнами — focus, unfocus, archive.

Расширяет lifecycle v1 из Phase 4.

- [ ] Focus/unfocus: управление какие окна в "контексте"
    - `focus <window>` — окно развёрнуто, content попадёт в промпт
    - `unfocus <window>` — окно свёрнуто (summary)
- [ ] Archive: frozen → archived (не в контексте, не в RAM)
    - `archive <id>` — frozen → archived
- [ ] `windows` команда — список с состояниями, exit code для frozen
- [ ] **Checkpoint**: `launch bash` → `launch python` → `focus 1` → `unfocus 2` → промпт показывает content окна 1, summary окна 2 → `archive 2` → окна 2 нет в промпте
- [ ] Тесты
- [ ] Doc-комментарии

### Phase 7: Prompt Assembly (full)

Цель: слои, бюджет, layout trait, references.

Расширяет плоский prompt v1 из Phase 4.

- [ ] `prompt/layout.rs` — PromptLayout trait + дефолтная реализация
    - Immutable layer: system prompt + mind.md
    - Ephemeral layer: dashboard + focused windows + previous response
    - System layer: status bar (tokens, windows, write target)
- [ ] `prompt/budget.rs` — подсчёт токенов (tiktoken-rs), бюджет слоёв
- [ ] `prompt/assembler.rs` — сборка промпта через PromptLayout
- [ ] `prompt/references.rs` — сбор --help/man page для окон с правом записи
- [ ] Команда `prompt` в Model Channel → показывает собранный промпт (debug)
- [ ] Обновить снепшот тесты — новая структура слоёв
- [ ] **Checkpoint**: открываю несколько окон → `prompt` → вижу слоёный промпт с budget
- [ ] Doc-комментарии

### Phase 8: Real LLM Client

Цель: подключаем реальную модель вместо stub/mock.

- [ ] `models/l_model.rs` — LLM клиент через llm crate (Claude/GPT API)
    - Реализация Agent trait
    - Thinking → mind.md (extended thinking extraction)
- [ ] Обработка ошибок: изоляция между блоками, timeout (30с)
- [ ] Idle timeout trigger (default 60s) — статусный тик
- [ ] User Viewport: debug view — что модель решила, что выполнилось
- [ ] **Checkpoint**: пишу в Kitty "найди все TODO" → Claude/GPT думает → открывает grep → результат → докладывает
- [ ] Doc-комментарии

### Phase 9: TUI Polish

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

### Phase 10: Session Persistence

Цель: перезапускаю `tai server` → frozen данные на месте, история сохранена.

- [ ] `session/manifest.rs` — session.json read/write (window registry, metadata)
- [ ] `session/snapshot.rs` — frozen content save/load на диск
- [ ] Graceful shutdown: freeze all active → save manifest → kill Kitty
- [ ] Recovery при старте: load manifest → reconnect/recreate windows
- [ ] **Checkpoint**: working session с frozen окнами → Ctrl+C → `tai server` → frozen данные доступны
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
