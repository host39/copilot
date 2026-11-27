//! Registry of application tables and what the `readonly` role may read from them.
//!
//! `migrate` revokes `SELECT` on every table, then grants it back only for tables listed here, and
//! never on lib-rustauth's `auth` schema. A table that is not listed stays unreadable, so register
//! a new table here only when nothing in it is private.

/// A column the readonly role must not read, with the reason (for example a secret or token).
#[derive(Clone, Copy, Debug)]
pub struct OmittedColumn {
    pub name: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct TableDoc {
    pub name: &'static str,
    pub summary: &'static str,
    pub omitted_columns: &'static [OmittedColumn],
}

/// The tables the readonly role may read (docs/design/copilot.md, "Readonly Role"). Every other
/// table, and every column listed in `omitted_columns`, stays unreadable.
pub const SCHEMA_TABLES: &[TableDoc] = &[
    TableDoc {
        name: "providers",
        summary: "A user's connection to a model provider (kind, preset, API, name).",
        omitted_columns: &[
            OmittedColumn {
                name: "base_url",
                reason: "An address may carry a credential in its path.",
            },
            OmittedColumn {
                name: "api_key_sealed",
                reason: "Sealed API key.",
            },
        ],
    },
    TableDoc {
        name: "models",
        summary: "Models a provider refresh found, with detected capabilities and overrides.",
        omitted_columns: &[],
    },
    TableDoc {
        name: "tool_servers",
        summary: "A user's MCP servers, with their tool modes.",
        omitted_columns: &[
            OmittedColumn {
                name: "url",
                reason: "Remote MCP addresses often carry a credential in their path.",
            },
            OmittedColumn {
                name: "headers_sealed",
                reason: "Sealed bearer token and custom headers.",
            },
            OmittedColumn {
                name: "tools_cached",
                reason: "The server's tool list, which may describe private systems.",
            },
        ],
    },
    TableDoc {
        name: "schedules",
        summary: "Prompts that run on a cron rule: timing, run state, and failure counts.",
        omitted_columns: &[
            OmittedColumn {
                name: "name",
                reason: "Written by the user, or by the assistant from a chat.",
            },
            OmittedColumn {
                name: "prompt",
                reason: "The prompt a run sends: user content.",
            },
            OmittedColumn {
                name: "instructions",
                reason: "A standalone schedule's instructions: user content.",
            },
            OmittedColumn {
                name: "last_error",
                reason: "May quote provider or tool output from a run.",
            },
        ],
    },
    TableDoc {
        name: "files",
        summary: "File metadata (type, size, estimates); the bytes are in file_contents.",
        omitted_columns: &[
            OmittedColumn {
                name: "name",
                reason: "The uploaded file's name: user content.",
            },
            OmittedColumn {
                name: "sha256",
                reason: "Would confirm whether a user holds a known file.",
            },
        ],
    },
];
