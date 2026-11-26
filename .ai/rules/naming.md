# Naming

Applies to new code and to code you touch. Do not mass-rename existing code unless asked. Never rename contract-bound names: API/JSON fields, DB columns, env vars, route params, serialized or snapshot keys.

## No "From" or "Of" in Function Names

Do not use `From` or `Of` as a connector in function or method names. Let the parameter types carry that meaning, or pick a verb that already implies it.

```rust
// BAD
fn widget_from_row(row: WidgetRow) -> Widget {}
fn total_of_items(items: &[Item]) -> i64 {}
fn parse_date_from_string(value: &str) -> NaiveDate {}

// GOOD
impl From<WidgetRow> for Widget {} // or fn to_widget(row: WidgetRow) -> Widget
fn sum(items: &[Item]) -> i64 {}
fn parse_date(value: &str) -> NaiveDate {}
```

```typescript
// BAD
function widgetFromResponse(body: WidgetResponse): Widget {}

// GOOD
function toWidget(body: WidgetResponse): Widget {}
```

Trait methods that std defines (`From::from`, `FromStr::from_str`, `TryFrom::try_from`) are exempt, and implementing those traits beats a free `x_from_y` function. Constructors that read configuration are `parse` or `load` (`AppConfig::parse(&env)`, `Env::load()`), not `from_env`.

## Keep Names Short

- Aim for 1 to 3 words. Four only when genuinely needed; more than four is always too long.
- Rely on the surrounding module, type, and parameter types for context instead of repeating it in the name.
- Drop words the module or type already makes clear (`WidgetService::get_widget` becomes `WidgetService::get`; `ApiContext::create`, not `create_api_context`).
- No type suffixes the type already says (`users_vec`, `config_struct`, `itemsArray`).
- Prefer a precise verb over a verb plus qualifiers (`sum`, `parse`, `to_x`).
- Short does not mean cryptic: whole words, no invented abbreviations, no single letters except loop indexes and very short closures.
- Follow each language's casing: Rust `snake_case` functions, modules, and files, `UpperCamelCase` types, `SCREAMING_SNAKE_CASE` constants; TypeScript `camelCase` functions and `PascalCase` components.
