# Markdown Title Case

Use **title case** for all Markdown headings (`#` through `######`). Browser tab titles follow the `document-titles` rule instead.

## Rules

- Capitalize major words (nouns, verbs, adjectives, adverbs).
- Lowercase short articles, conjunctions, and prepositions (`a`, `an`, `the`, `and`, `but`, `or`, `for`, `in`, `of`, `on`, `to`, `with`, `vs`) unless they are the first or last word.
- Capitalize both parts of hyphenated compounds (`End-to-End`, `Non-Goals`).
- Keep acronyms, crate names, and package names as written (`ADR`, `HTTP`, `sqlx`, `copilot-core`, `@copilot/web`).
- **Counterexamples:** the BAD headings in the Examples section below are intentional. Do not "fix" them when applying this rule.

## Examples

```markdown
<!-- BAD -->
## Package map
### Server vs lambda
## Further reading

<!-- GOOD -->
## Package Map
### Server vs Lambda
## Further Reading
```
