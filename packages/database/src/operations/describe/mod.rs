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

pub const SCHEMA_OVERVIEW: &str = "Copilot's tables live in `public`; lib-rustauth's accounts and \
     sessions live in `auth`, which the readonly role cannot read. The readonly role reads \
     provider connections, models, tool servers, schedules, and file metadata, without sealed \
     credentials. Instructions, projects, chats, messages, turn snapshots, file bytes, and the \
     join tables stay private.";

/// The tables the readonly role may read (docs/design/copilot.md, "Readonly Role"). Every other
/// table, and every column listed in `omitted_columns`, stays unreadable.
pub const SCHEMA_TABLES: &[TableDoc] = &[
    TableDoc {
        name: "providers",
        summary: "A user's connection to a model provider (kind, preset, name, base URL).",
        omitted_columns: &[OmittedColumn {
            name: "api_key_sealed",
            reason: "Sealed API key.",
        }],
    },
    TableDoc {
        name: "models",
        summary: "Models a provider refresh found, with detected capabilities and overrides.",
        omitted_columns: &[],
    },
    TableDoc {
        name: "tool_servers",
        summary: "A user's MCP servers, with tool modes and the cached tool list.",
        omitted_columns: &[OmittedColumn {
            name: "headers_sealed",
            reason: "Sealed bearer token and custom headers.",
        }],
    },
    TableDoc {
        name: "schedules",
        summary: "Prompts that run on a cron rule, with run state and failure counts.",
        omitted_columns: &[],
    },
    TableDoc {
        name: "files",
        summary: "File metadata (name, type, size, estimates); the bytes are in file_contents.",
        omitted_columns: &[],
    },
];
