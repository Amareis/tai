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

### Phase 2: FD Passing + Minimal Dual TUI

Цель: `tai server` → Kitty открывается → оба терминала рисуют.

- [x] Добавить зависимости: `roam-fdpass` (для SCM_RIGHTS)
- [x] `fd/mod.rs` — Unix Domain Socket server/client
    - Server: listen на `/tmp/tai.sock`, accept connection, recv FD через `recvmsg` + `SCM_RIGHTS`
    - Client: connect, send FD (stdout) через `sendmsg` + `SCM_RIGHTS`, sleep
- [ ] `fd/terminal_manager.rs` — TerminalManager
    - `HashMap<ViewId, Terminal<CrosstermBackend<File>>>`
    - Создание terminal на FD (User viewport при старте, Model viewport при подключении client)
    - `draw_all()` — перерисовка всех viewport'ов
- [x] Обновить `main.rs` — clap subcommands:
    - `tai server [--socket PATH] [--hidden]` — инициализирует User Viewport на stdout,
      spawn Kitty с `kitty -- tai client --socket PATH`, ждёт FD, создаёт Model Viewport
    - `tai client --socket PATH` — подключиться к сокету, передать FD, sleep
- [ ] SIGWINCH handler в client → resize message через Unix socket → server вызывает `terminal.resize()`
- [x] Минимальная отрисовка: User Viewport = "TAI Server (User)", Model Viewport = "TAI Model Workspace"
- [ ] **Checkpoint**: запускаю `tai server` → Kitty открывается → оба окна показывают текст → resize работает
- [ ] Doc-комментарии

### Phase 3: Command Parser + Window Operations

Цель: печатаю `launch bash` в Kitty окно → появляется новое окно → вижу в User Viewport.

- [ ] `routing/parser.rs` — парсинг текстовых команд из Model Viewport
    - Формат: `launch --title name -- cmd args`, `close <window-id>`, `focus <window-id>`,
      `list`, `send <window-id> text...`
    - Простой line-based парсер (не code blocks — это для модели, человек пишет plain commands)
- [ ] `routing/tai_command.rs` — TaiCommand enum, парсер команд
- [ ] Чтение ввода из Model Viewport (crossterm events → command parser → dispatch)
- [ ] Dispatch: tai_command → backend calls (launch, close, send-text, list-windows, get-text)
- [ ] User Viewport: список окон (обновляется после каждой команды)
- [ ] Model Viewport: результат команды (stdout-style feedback: "launched window abc123")
- [ ] **Checkpoint**: в Kitty окне набираю `launch bash` → появляется Kitty tab → `list` → вижу оба окна → `close 1` → окно закрылось
- [ ] Тесты парсера (валидные/невалидные команды)
- [ ] Doc-комментарии

### Phase 4: Window Lifecycle + Session Persistence

Цель: перезапускаю `tai server` → окна восстанавливаются, frozen данные на месте.

- [ ] `session/manager.rs` — Window lifecycle (Active → Frozen → Archived)
    - at_prompt detection → freeze → get-text → snapshot → close
    - Focus/summarize: управление какие окна в "контексте"
- [ ] `session/manifest.rs` — session.json read/write (window registry, metadata)
- [ ] `session/snapshot.rs` — frozen content save/load
- [ ] Graceful shutdown: freeze all active → save manifest → kill Kitty
- [ ] Recovery при старте: load manifest → reconnect/recreate windows
- [ ] **Checkpoint**: запускаю `launch bash -c "echo hello && sleep 5"` → команда выполняется → окно freezes → exit code + вывод сохранены → рестарт tai → frozen данные доступны
- [ ] Тесты lifecycle с MockBackend
- [ ] Doc-комментарии

### Phase 5: Prompt Assembly

Цель: вижу собранный промпт в debug output. Реальные данные из окон.

- [ ] `prompt/layout.rs` — PromptLayout trait + дефолтная реализация
    - Immutable layer: system prompt + mind.md
    - Ephemeral layer: dashboard + focused windows + previous response
    - System layer: status bar (tokens, windows, write target)
- [ ] `prompt/budget.rs` — подсчёт токенов (tiktoken-rs), бюджет слоёв
- [ ] `prompt/assembler.rs` — сборка промпта через PromptLayout
- [ ] `prompt/references.rs` — сбор --help/man page для окон с правом записи
- [ ] Model Viewport: команда `prompt` → показывает собранный промпт (для debug)
- [ ] **Checkpoint**: открываю несколько окон → `prompt` → вижу полный промпт с содержимым окон, mind, budget
- [ ] Тесты layout (mock данные, проверка структуры и бюджета)
- [ ] Doc-комментарии

### Phase 6: L-Model + Event Loop

Цель: полный цикл. Пишу в Kitty → модель отвечает → ядро исполняет → результат виден.

- [ ] `models/l_model.rs` — LLM клиент через llm crate (Claude/GPT API)
- [ ] Расширить parser: модель отвечает markdown с code blocks (```window:mode```)
    - Человек: plain commands (из Phase B)
    - Модель: code blocks с обязательными window:mode
    - Один парсер, два input format
- [ ] `kernel/mod.rs` — main tick loop
    - Триггеры: at_prompt (command done) / user input in Model Viewport / idle timeout
    - Tick: assemble → invoke model → parse response → execute blocks → wait
    - Blocks выполняются параллельно
- [ ] Thinking → mind.md (извлечение из extended thinking)
- [ ] Обработка ошибок: изоляция между блоками, timeout (30с)
- [ ] Model Viewport: показывает ответы модели (prose + executed blocks)
- [ ] User Viewport: debug view — что модель решила, что выполнилось
- [ ] **Checkpoint**: пишу в Kitty "найди все TODO в проекте" → модель открывает окно с grep → результат виден → модель докладывает
- [ ] Doc-комментарии

### Phase 7: TUI Polish

Цель: полноценные интерактивные интерфейсы в обоих viewport'ах.

- [ ] Model Viewport — Chat UI:
    - Scrollable история, подсветка code blocks, input field
    - Context view: collapsible focused windows
- [ ] User Viewport — Debug UI:
    - Windows tab: список с фильтрами, preview frozen content
    - Debug tab: пошаговое исполнение (Step / Run All / Edit / Skip)
    - Status bar: токены, активные окна, write target, tick count
- [ ] `tui/status_bar.rs` — общие компоненты
- [ ] Обработка горячих клавиш в обоих viewport'ах
- [ ] **Checkpoint**: полноценная интерактивная сессия — model view как IDE, user view как dashboard

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

## S-Models

Лёгкие модели-наблюдатели для свёрнутых окон. Система полностью работает без них.

## IPC / Remote API 

Unix socket или HTTP API для внешних клиентов. Когда появится TmuxBackend или remote.
