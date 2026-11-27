//! stdio tool servers: a program on the API host that speaks MCP over its stdin and stdout
//! (`docs/design/copilot.md`, stdio Servers).
//!
//! Only administrators add, change, and run them, and only while `MCP_STDIO_ENABLED` is on; both
//! are checked when a server is saved and again right before every process starts
//! ([`StdioGate`]). The program runs directly with its argument list, never through a shell,
//! with only the environment variables entered here (sealed in `env_sealed`), in its own
//! directory under `MCP_STDIO_DIR`.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ports::accounts::Accounts;
use crate::ports::tool_host::SpawnCheck;
use crate::shared::actor::Actor;
use crate::shared::constants::{
    STDIO_ARG_MAX_CHARS, STDIO_ARGS_MAX, STDIO_COMMAND_MAX_CHARS, STDIO_CWD_MAX_CHARS,
    STDIO_ENV_MAX, STDIO_ENV_NAME_MAX_CHARS, STDIO_ENV_VALUE_MAX_CHARS,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ToolServerId, UserId};
use crate::shared::secret::{REDACTED, Secret};

/// The error code when `MCP_STDIO_ENABLED` is off.
pub const STDIO_DISABLED: &str = "STDIO_DISABLED";

/// How Copilot reaches a tool server (`tool_servers.transport`). Set when the server is added;
/// it never changes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Transport {
    /// Streamable HTTP at the server's URL.
    #[default]
    Http,
    /// A program on the API host, over its stdin and stdout.
    Stdio,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Stdio => "stdio",
        }
    }
}

impl FromStr for Transport {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "http" => Ok(Self::Http),
            "stdio" => Ok(Self::Stdio),
            other => Err(other.to_owned()),
        }
    }
}

/// A stdio server's program as stored: `command`, `args`, `env_sealed` (bound to the row, see
/// `ToolServer::env_context`), and `cwd`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StdioCommand {
    /// The program's absolute path.
    pub command: String,
    /// Passed to the program as they are: no shell, no expansion.
    pub args: Vec<String>,
    /// The sealed [`StdioEnv`]; `None` starts the process with an empty environment.
    pub env_sealed: Option<Vec<u8>>,
    /// The working directory, relative to `MCP_STDIO_DIR`.
    pub cwd: String,
}

/// A stdio server's program as an administrator enters it. `Debug` hides the environment's
/// values.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct StdioInput {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Blank or absent: the server's id.
    pub cwd: Option<String>,
}

impl fmt::Debug for StdioInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StdioInput")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &Names(&self.env))
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// A change to a stdio server's program; `None` keeps a field. `Debug` hides the environment's
/// values.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct StdioUpdate {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    /// Replaces every variable; an empty map removes them all.
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<String>,
}

impl StdioUpdate {
    pub fn is_empty(&self) -> bool {
        self.command.is_none() && self.args.is_none() && self.env.is_none() && self.cwd.is_none()
    }
}

impl fmt::Debug for StdioUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StdioUpdate")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &self.env.as_ref().map(Names))
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// Variable names with `***` for each value.
struct Names<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for Names<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(self.0.keys().map(|name| (name, REDACTED)))
            .finish()
    }
}

/// The environment of a stdio server's process: the only variables it gets (the API's own
/// environment, with `DATABASE_URL`-style settings and `SECRET_KEY`, never reaches it). Sealed as
/// one JSON object of name to value (`env_sealed`). `Debug` prints the names only.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct StdioEnv {
    entries: BTreeMap<String, String>,
}

impl StdioEnv {
    /// Validates the form input: names are letters, digits, and `_`, not starting with a digit;
    /// values are kept exactly as entered (no trimming) and may not contain a NUL byte.
    pub fn new(variables: &BTreeMap<String, String>) -> DomainResult<Self> {
        if variables.len() > STDIO_ENV_MAX {
            return Err(invalid_env(format!(
                "A stdio server takes at most {STDIO_ENV_MAX} environment variables."
            )));
        }
        let mut entries = BTreeMap::new();
        for (name, value) in variables {
            let name = name.trim();
            let valid = !name.is_empty()
                && name.chars().count() <= STDIO_ENV_NAME_MAX_CHARS
                && !name.starts_with(|character: char| character.is_ascii_digit())
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_');
            if !valid {
                return Err(invalid_env(format!(
                    "Environment variable names are letters, digits, and _ (not starting with a digit), at most {STDIO_ENV_NAME_MAX_CHARS} characters."
                )));
            }
            if value.contains('\0') || value.chars().count() > STDIO_ENV_VALUE_MAX_CHARS {
                return Err(invalid_env(format!(
                    "The value of {name} must be at most {STDIO_ENV_VALUE_MAX_CHARS} characters, without NUL characters."
                )));
            }
            if entries.insert(name.to_owned(), value.clone()).is_some() {
                return Err(invalid_env(format!("{name} is listed twice.")));
            }
        }
        Ok(Self { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The JSON object that gets sealed.
    pub fn to_json(&self) -> Vec<u8> {
        let object: Map<String, Value> = self
            .entries
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        serde_json::to_vec(&Value::Object(object)).expect("a string map serializes")
    }

    /// The variables of an opened secret, for the MCP adapter.
    pub fn entries(secret: &Secret) -> DomainResult<Vec<(String, String)>> {
        let invalid = || DomainError::Internal {
            key: "core.stdio-env.invalid".to_owned(),
            source: None,
        };
        let value: Value = serde_json::from_slice(secret.expose()).map_err(|_| invalid())?;
        let Value::Object(object) = value else {
            return Err(invalid());
        };
        object
            .into_iter()
            .map(|(name, value)| match value {
                Value::String(text) => Ok((name, text)),
                _ => Err(invalid()),
            })
            .collect()
    }

    /// The variable names of an opened secret, for the settings.
    pub fn names(secret: &Secret) -> DomainResult<Vec<String>> {
        Ok(Self::entries(secret)?
            .into_iter()
            .map(|(name, _)| name)
            .collect())
    }
}

impl fmt::Debug for StdioEnv {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_set().entries(self.entries.keys()).finish()
    }
}

fn invalid_env(message: String) -> DomainError {
    DomainError::validation(message).with_field("stdio.env")
}

/// The program path: absolute, one line, at most `STDIO_COMMAND_MAX_CHARS`. Relative names are
/// refused because the process starts without `PATH`, so nothing would look them up.
pub(crate) fn command(text: &str) -> DomainResult<String> {
    let command = text.trim();
    let invalid = |message: &str| DomainError::validation(message).with_field("stdio.command");
    if command.is_empty() {
        return Err(invalid("Enter the program's path."));
    }
    if !command.starts_with('/') {
        return Err(invalid(
            "Enter the program's absolute path (for example /usr/local/bin/my-server): it starts without PATH, so names are not looked up.",
        ));
    }
    if command.chars().count() > STDIO_COMMAND_MAX_CHARS || command.chars().any(char::is_control) {
        return Err(invalid(&format!(
            "The program's path must be one line of at most {STDIO_COMMAND_MAX_CHARS} characters."
        )));
    }
    Ok(command.to_owned())
}

/// The arguments, kept exactly as entered: at most `STDIO_ARGS_MAX`, each at most
/// `STDIO_ARG_MAX_CHARS` and without a NUL character.
pub(crate) fn args(list: &[String]) -> DomainResult<Vec<String>> {
    if list.len() > STDIO_ARGS_MAX {
        return Err(DomainError::validation(format!(
            "A stdio server takes at most {STDIO_ARGS_MAX} arguments."
        ))
        .with_field("stdio.args"));
    }
    if list
        .iter()
        .any(|arg| arg.contains('\0') || arg.chars().count() > STDIO_ARG_MAX_CHARS)
    {
        return Err(DomainError::validation(format!(
            "Each argument must be at most {STDIO_ARG_MAX_CHARS} characters, without NUL characters."
        ))
        .with_field("stdio.args"));
    }
    Ok(list.to_vec())
}

/// The working directory under `MCP_STDIO_DIR`: a relative path of plain names (letters,
/// digits, `.`, `_`, `-`; never `.` or `..`) joined by `/`, so it cannot leave the base. Blank
/// means `default`.
pub(crate) fn cwd(text: Option<&str>, default: &str) -> DomainResult<String> {
    let cwd = text.map(str::trim).filter(|cwd| !cwd.is_empty());
    let Some(cwd) = cwd else {
        return Ok(default.to_owned());
    };
    let plain = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
    };
    if cwd.chars().count() > STDIO_CWD_MAX_CHARS || !cwd.split('/').all(plain) {
        return Err(DomainError::validation(format!(
            "The working directory is a folder inside the stdio base directory: names of letters, digits, ., _, and - joined by /, at most {STDIO_CWD_MAX_CHARS} characters, without . or .. parts."
        ))
        .with_field("stdio.cwd"));
    }
    Ok(cwd.to_owned())
}

/// `MCP_STDIO_ENABLED` and the accounts that say whether a user is an administrator now.
#[derive(Clone)]
pub struct StdioPolicy {
    pub enabled: bool,
    pub accounts: Arc<dyn Accounts>,
}

impl StdioPolicy {
    /// Whether `actor` may add and run stdio servers, as far as the request knows: the setting
    /// is on and the session's account is an administrator.
    pub fn allows(&self, actor: &Actor) -> bool {
        self.enabled && actor.is_admin
    }

    /// Fails unless [`StdioPolicy::allows`]: `STDIO_DISABLED` (422) when the setting is off,
    /// `FORBIDDEN` (403) for anyone but an administrator.
    pub fn check(&self, actor: &Actor) -> DomainResult<()> {
        if !self.enabled {
            return Err(disabled());
        }
        if !actor.is_admin {
            return Err(admin_only());
        }
        Ok(())
    }
}

pub(crate) fn disabled() -> DomainError {
    DomainError::business_rule(
        "Tool servers that run a program on this server are turned off (MCP_STDIO_ENABLED).",
        STDIO_DISABLED,
    )
}

pub(crate) fn admin_only() -> DomainError {
    DomainError::forbidden(
        "Only administrators can add, change, or run tool servers that run a program on this server.",
    )
}

/// The check before every start of a server's process: the setting, then the owner's account as
/// it is now (one read), so a demoted or disabled administrator's server stops starting at once.
pub(crate) struct StdioGate {
    pub(crate) policy: StdioPolicy,
    pub(crate) user: UserId,
    pub(crate) server: ToolServerId,
}

#[async_trait]
impl SpawnCheck for StdioGate {
    async fn allow(&self) -> DomainResult<()> {
        if !self.policy.enabled {
            tracing::warn!(server_id = %self.server, reason = "disabled", "core.tools.stdio.refused");
            return Err(disabled());
        }
        let actors = self.policy.accounts.active(&[self.user]).await?;
        let admin = actors
            .iter()
            .any(|actor| actor.user_id == self.user && actor.is_admin);
        if !admin {
            tracing::warn!(server_id = %self.server, reason = "not-admin", "core.tools.stdio.refused");
            return Err(admin_only());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use uuid::Uuid;

    use super::*;
    use crate::testing::{FixedClock, MemoryStore};

    /// The memory store's accounts, counting reads.
    struct Counted {
        store: MemoryStore,
        reads: AtomicUsize,
    }

    impl Counted {
        fn new() -> Self {
            Self {
                store: MemoryStore::new(Arc::new(FixedClock::default())),
                reads: AtomicUsize::new(0),
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Accounts for Counted {
        async fn active(&self, ids: &[UserId]) -> DomainResult<Vec<Actor>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.store.active(ids).await
        }
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn transport_round_trips() {
        for transport in [Transport::Http, Transport::Stdio] {
            assert_eq!(transport.as_str().parse::<Transport>(), Ok(transport));
        }
        assert!("ws".parse::<Transport>().is_err());
        assert_eq!(Transport::default(), Transport::Http);
    }

    #[test]
    fn the_program_is_an_absolute_path_on_one_line() {
        assert_eq!(
            command(" /usr/local/bin/server ").unwrap(),
            "/usr/local/bin/server"
        );
        for bad in [
            "",
            "  ",
            "npx",
            "./server",
            "bin/server",
            "/bin/a\nb",
            "/bin/\0",
        ] {
            let error = command(bad).unwrap_err();
            assert!(
                matches!(&error, DomainError::Validation { field: Some(field), .. } if field == "stdio.command"),
                "{bad:?}: {error:?}"
            );
        }
        let long = format!("/{}", "a".repeat(STDIO_COMMAND_MAX_CHARS));
        assert!(command(&long).is_err());
    }

    #[test]
    fn arguments_are_kept_exactly() {
        let list = vec![
            " --flag ".to_owned(),
            "a b; rm -rf /".to_owned(),
            String::new(),
        ];
        assert_eq!(args(&list).unwrap(), list, "no trimming, no shell");
        assert!(args(&["a\0b".to_owned()]).is_err());
        assert!(args(&vec!["a".to_owned(); STDIO_ARGS_MAX + 1]).is_err());
        assert!(args(&["a".repeat(STDIO_ARG_MAX_CHARS + 1)]).is_err());
        assert!(args(&vec!["a".to_owned(); STDIO_ARGS_MAX]).is_ok());
    }

    #[test]
    fn the_directory_stays_under_the_base() {
        assert_eq!(cwd(None, "fallback").unwrap(), "fallback");
        assert_eq!(cwd(Some("  "), "fallback").unwrap(), "fallback");
        assert_eq!(cwd(Some("github"), "x").unwrap(), "github");
        assert_eq!(
            cwd(Some("team/git-hub_1.0"), "x").unwrap(),
            "team/git-hub_1.0"
        );
        for bad in [
            "/etc", "..", "../x", "a/../b", "a/./b", ".", "a//b", "a/", "a b", "a\\b", "~", "$HOME",
        ] {
            assert!(cwd(Some(bad), "x").is_err(), "{bad}");
        }
        assert!(cwd(Some(&"a".repeat(STDIO_CWD_MAX_CHARS + 1)), "x").is_err());
    }

    #[test]
    fn env_names_are_checked_and_values_kept() {
        let env = StdioEnv::new(&map(&[(" API_KEY ", " secret value "), ("_X1", "")])).unwrap();
        let opened = Secret::new(env.to_json());
        assert_eq!(
            StdioEnv::entries(&opened).unwrap(),
            [
                ("API_KEY".to_owned(), " secret value ".to_owned()),
                ("_X1".to_owned(), String::new()),
            ]
        );
        assert_eq!(StdioEnv::names(&opened).unwrap(), ["API_KEY", "_X1"]);
        for bad in ["", "1ABC", "A-B", "A B", "A=B", "Ä"] {
            assert!(StdioEnv::new(&map(&[(bad, "v")])).is_err(), "{bad}");
        }
        assert!(StdioEnv::new(&map(&[("A", "x\0y")])).is_err());
        assert!(
            StdioEnv::new(&map(&[("A", " "), (" A", "")])).is_err(),
            "twice"
        );
        assert!(StdioEnv::new(&BTreeMap::new()).unwrap().is_empty());
    }

    #[test]
    fn debug_never_prints_env_values() {
        let variables = map(&[("API_KEY", "sk-TOPSECRET")]);
        let input = StdioInput {
            command: "/bin/server".to_owned(),
            env: variables.clone(),
            ..StdioInput::default()
        };
        let update = StdioUpdate {
            env: Some(variables.clone()),
            ..StdioUpdate::default()
        };
        let env = StdioEnv::new(&variables).unwrap();
        for printed in [
            format!("{input:?}"),
            format!("{update:?}"),
            format!("{env:?}"),
        ] {
            assert!(!printed.contains("TOPSECRET"), "{printed}");
            assert!(printed.contains("API_KEY"), "{printed}");
        }
    }

    #[test]
    fn the_policy_needs_the_setting_and_an_administrator() {
        let user = UserId::new(Uuid::now_v7());
        let accounts = Arc::new(Counted::new());
        let on = StdioPolicy {
            enabled: true,
            accounts: accounts.clone(),
        };
        let off = StdioPolicy {
            enabled: false,
            accounts,
        };
        assert!(on.allows(&Actor::admin(user)));
        assert!(!on.allows(&Actor::user(user)));
        assert!(!off.allows(&Actor::admin(user)));
        assert_eq!(
            on.check(&Actor::user(user)).unwrap_err().code(),
            "FORBIDDEN"
        );
        assert_eq!(
            off.check(&Actor::admin(user)).unwrap_err().code(),
            STDIO_DISABLED
        );
        assert_eq!(
            off.check(&Actor::user(user)).unwrap_err().code(),
            STDIO_DISABLED,
            "the setting first"
        );
    }

    #[tokio::test]
    async fn the_gate_reads_the_account_at_every_start() {
        let user = UserId::new(Uuid::now_v7());
        let accounts = Arc::new(Counted::new());
        let gate = StdioGate {
            policy: StdioPolicy {
                enabled: true,
                accounts: accounts.clone(),
            },
            user,
            server: ToolServerId::generate(),
        };
        let error = gate.allow().await.unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN", "a plain user");
        accounts.store.add_user(&Actor::admin(user));
        gate.allow().await.expect("an administrator now");
        accounts.store.add_user(&Actor::user(user));
        assert_eq!(
            gate.allow().await.unwrap_err().code(),
            "FORBIDDEN",
            "demoted"
        );
        accounts.store.add_user(&Actor::admin(user));
        accounts.store.disable_user(user);
        assert_eq!(
            gate.allow().await.unwrap_err().code(),
            "FORBIDDEN",
            "disabled"
        );
        assert_eq!(accounts.reads(), 4, "one read per start");

        let off = StdioGate {
            policy: StdioPolicy {
                enabled: false,
                accounts: accounts.clone(),
            },
            user,
            server: ToolServerId::generate(),
        };
        assert_eq!(off.allow().await.unwrap_err().code(), STDIO_DISABLED);
        assert_eq!(accounts.reads(), 4, "the setting is checked first");
    }
}
