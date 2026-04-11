# Plan: File Editing in TAI

## Проблема

Сейчас TAI поддерживает три режима для code blocks:

- **`:text`** — запускает команду в новом окне (shell). Можно использовать `sed`, `awk`, `patch` и т.д., но prone to quoting/escaping issues.
- **`:close`** — закрывает окно по заголовку.
- **`:write<<DELIM`** — перезаписывает файл целиком. Надёжно, но не позволяет сделать точечное изменение в большом файле.

**Редактирование** — это точечное изменение существующего файла без полной перезаписи. Это критически важно, потому что:

1. `:write` требует пересоздавать весь файл — долго для больших файлов, много токенов.
2. `sed` через `:text` — хрупко (quoting, escaping, нет реальной проверки что изменение применилось).
3. LLM видит содержимое файлов в промпте и может определить точный текст для замены.

## Решение: режим `:edit`

Добавить новый `BlockMode::Edit` с search-and-replace синтаксисом. Модель видит файл в промпте и может указать точный фрагмент для замены.

### Формат

````
```src/main.rs:edit<<DELIM
<<<<<<< SEARCH
old line 1
old line 2
=======
new line 1
new line 2
>>>>>>> REPLACE
DELIM
````

**Множественные замены** в одном блоке:

````
```src/main.rs:edit<<DELIM
<<<<<<< SEARCH
fn old_function() -> i32 {
    42
}
=======
fn new_function() -> String {
    "hello".to_string()
}
>>>>>>> REPLACE

<<<<<<< SEARCH
use crate::old_module;
=======
use crate::new_module;
>>>>>>> REPLACE
DELIM
````

### Свойства формата

- **Точный поиск** — старый текст должен совпадать буквально (включая пробелы и переносы строк). Это не regex, не glob — точное совпадение.
- **Однозначность** — если найдено >1 совпадение, это ошибка (неоднозначное изменение). Модель должна включить больше контекста.
- **Не найдено** — если текст не найден, ошибка возвращается как feedback.
- **Heredoc-обёртка обязательна** — содержимое может содержать triple backticks, поэтому `<<DELIM` необходим.
- **Несколько блоков SEARCH/REPLACE** — в одном edit block можно сделать несколько независимых замен. Они применяются последовательно.

### Маркеры

| Маркер | Назначение |
|--------|-----------|
| `<<<<<<< SEARCH` | Начало текста для поиска |
| `=======` | Разделитель: после — новый текст |
| `>>>>>>> REPLACE` | Конец блока замены |

Выбран как устоявшийся формат (aider, Cursor и др.) — LLM привычны к нему.

## Реализация

### 1. `src/types.rs` — добавить `BlockMode::Edit`

```rust
pub enum BlockMode {
    Text,
    Close,
    Write,
    Edit,  // NEW
}
```

`FromStr` для `"edit"` → `BlockMode::Edit`, `Display` для `Edit` → `"edit"`.

### 2. `src/response/mod.rs` — парсинг edit blocks

Edit blocks **всегда** используют heredoc (как `:write`). Парсер уже поддерживает heredoc — `parse_block_header` распарсит `src/main.rs:edit<<DELIM` и установит `mode = Edit` + `heredoc_delim = Some("DELIM")`.

Никаких изменений в парсере не нужно — он уже обрабатывает heredoc и корректно захватывает содержимое с triple backticks внутри.

### 3. Новый модуль `src/edit.rs` — логика применения правок

```rust
/// Результат применения одной замены.
pub struct EditResult {
    pub found: usize,       // сколько раз найдено (должно быть 1)
    pub replaced: bool,     // успешно заменено
    pub message: String,    // для feedback
}

/// Ошибка при применении правок.
pub enum EditError {
    NotFound { search: String },
    MultipleMatches { search: String, count: usize },
    FileNotFound { path: String },
    IoError(std::io::Error),
}

/// Применить search/replace блоки к файлу.
///
/// Блоки применяются последовательно. Если любой блок не удался —
/// файл не записывается (atomistic: all-or-nothing).
pub fn apply_edits(path: &str, edits: &[(String, String)]) -> Result<Vec<EditResult>, EditError> {
    let content = std::fs::read_to_string(path)?;  // FileNotFound if missing
    let mut current = content;
    let mut results = Vec::new();
    
    for (search, replace) in edits {
        let count = current.matches(search).count();
        match count {
            0 => return Err(EditError::NotFound { search: search.clone() }),
            1 => {
                current = current.replacen(search, replace, 1);
                results.push(EditResult { found: 1, replaced: true, message: format!("replaced 1 occurrence") });
            }
            _ => return Err(EditError::MultipleMatches { search: search.clone(), count }),
        }
    }
    
    std::fs::write(path, &current)?;
    Ok(results)
}

/// Парсинг содержимого edit block в список (search, replace) пар.
///
/// Формат:
/// ```
/// <<<<<<< SEARCH
/// old text
/// =======
/// new text
/// >>>>>>> REPLACE
/// ```
pub fn parse_edit_content(content: &str) -> Vec<(String, String)> {
    // Разбить по <<<<<<< SEARCH маркерам
    // Для каждого блока: найти ======= и >>>>>>> REPLACE
    // Вернуть пары (search_text, replace_text)
}
```

Ключевые решения:
- **All-or-nothing** — если хоть один блок не найдён или неоднозначен, файл не меняется. Модель получит feedback и попробует снова.
- **Последовательное применение** — каждая замена применяется к результату предыдущей. Порядок важен.
- **Точное совпадение** — `matches()` для подсчёта, `replacen()` для замены. Без regex, без fuzzy matching.

### 4. `src/core/mod.rs` — обработка `BlockMode::Edit`

В `execute_block()` добавить ветку:

```rust
BlockMode::Edit => {
    let edits = parse_edit_content(content);
    let result = match apply_edits(title, &edits) {
        Ok(results) => {
            let count = results.len();
            format!("[{title}] {count} edit(s) applied")
        }
        Err(EditError::NotFound { search }) => {
            format!("[{title}] ERROR: search text not found:\n{}", truncate(&search, 200))
        }
        Err(EditError::MultipleMatches { search, count }) => {
            format!("[{title}] ERROR: ambiguous edit, found {count} matches:\n{}", truncate(&search, 200))
        }
        Err(EditError::IoError(e)) => {
            format!("[{title}] ERROR: {e}")
        }
    };
    ExecutedBlock {
        title: title.to_string(),
        window_id: None,
        mode: *mode,
        result: Some(result),
    }
}
```

### 5. Обновить system prompt в `src/prompt/mod.rs`

Добавить описание `:edit` режима:

```
```<file_path>:edit<<DELIM\n<<<<<<< SEARCH\nold text\n=======\nnew text\n>>>>>>> REPLACE\nDELIM\n``` — edit existing file using search/replace (no full rewrite needed)
```

И усилить инструкции:
- Prefer `:edit` over `:write` when making targeted changes to existing files.
- Use `:write` only for creating new files or completely rewriting small files.

### 6. Обновить `src/prompt/mod.rs` — `Prompt::to_text()`

Если `Prompt` сейчас рендерится в текст для LLM, добавить в вывод содержимое файлов-кандидатов для редактирования (с line numbers для удобства LLM).

Если `to_text()` ещё не существует — он будет добавлен в Phase 4/5. Сейчас промпт собирается в `Prompt::build()` как набор полей.

## Порядок реализации

| Шаг | Что | Сложность |
|-----|------|-----------|
| 1 | `BlockMode::Edit` в `types.rs` + `FromStr`/`Display` | 5 мин |
| 2 | `parse_edit_content()` в новом `src/edit.rs` + тесты | 20 мин |
| 3 | `apply_edits()` в `src/edit.rs` + тесты | 20 мин |
| 4 | `BlockMode::Edit` ветка в `execute_block()` | 10 мин |
| 5 | Обновить system prompt | 5 мин |
| 6 | Интеграционный тест | 10 мин |

## Тесты для `src/edit.rs`

```rust
#[test]
fn test_parse_single_edit() {
    let content = "<<<<<<< SEARCH\nold text\n=======\nnew text\n>>>>>>> REPLACE";
    let edits = parse_edit_content(content);
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].0, "old text\n");
    assert_eq!(edits[0].1, "new text\n");
}

#[test]
fn test_parse_multiple_edits() {
    let content = "<<<<<<< SEARCH\nfn old() {\n=======\nfn new() {\n>>>>>>> REPLACE\n\n<<<<<<< SEARCH\nuse old;\n=======\nuse new;\n>>>>>>> REPLACE";
    let edits = parse_edit_content(content);
    assert_eq!(edits.len(), 2);
}

#[test]
fn test_apply_edits_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.rs");
    fs::write(&path, "fn old() { 42 }\nfn other() { 99 }\n").unwrap();
    
    let edits = vec![
        ("fn old() { 42 }".to_string(), "fn new() { 0 }".to_string()),
    ];
    let results = apply_edits(path.to_str().unwrap(), &edits).unwrap();
    assert_eq!(results.len(), 1);
    
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("fn new() { 0 }"));
    assert!(!content.contains("fn old() { 42 }"));
}

#[test]
fn test_apply_edits_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.rs");
    fs::write(&path, "hello world\n").unwrap();
    
    let edits = vec![("not present".to_string(), "replacement".to_string())];
    assert!(matches!(
        apply_edits(path.to_str().unwrap(), &edits),
        Err(EditError::NotFound { .. })
    ));
    // File unchanged
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello world\n");
}

#[test]
fn test_apply_edits_multiple_matches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.rs");
    fs::write(&path, "dup\ndup\nunique\n").unwrap();
    
    let edits = vec![("dup".to_string(), "replaced".to_string())];
    assert!(matches!(
        apply_edits(path.to_str().unwrap(), &edits),
        Err(EditError::MultipleMatches { count: 2, .. })
    ));
}

#[test]
fn test_apply_edits_file_not_found() {
    let edits = vec![("old".to_string(), "new".to_string())];
    assert!(matches!(
        apply_edits("/nonexistent/file.rs", &edits),
        Err(EditError::IoError(_))
    ));
}
```

## Альтернативы, рассмотренные и отклонённые

### Unified diff / patch формат
`diff -u` формат требует точных line numbers и контекстных строк. LLM часто ошибается в нумерации строк. Search/replace надёжнее — LLM копирует то, что видит.

### sed через `:text`
Уже работает, но хрупко: quoting issues, нет обратной связи о том действительно ли изменение применилось, нет проверки на неоднозначность.

### Только `:write` (полная перезапись)
Работает, но дорого по токенам для больших файлов. Для файла в 500 строк ради замены 3 строк — пересоздание всего файла.

### Line-number based edits (`:10:replace:new text`)
Зависит от нумерации строк, которая может смещаться при множественных правках. Хрупко.

## Итого

`:edit` — это минимальное, надёжное расширение существующей системы. Оно:

1. **Не требует shell** — правки применяются в Rust коде, никаких escaping проблем.
2. **All-or-nothing** — атомарность: если хоть один блок не найдён, файл не меняется.
3. **LLM-friendly** — модель видит содержимое файлов в промпте, может копировать точный текст.
4. **Обратная связь** — чёткий feedback: "not found", "ambiguous (2 matches)", "applied 3 edits".
5. **Расширяет, не ломает** — `:write` остаётся для создания файлов и полных перезаписей.
EOF```