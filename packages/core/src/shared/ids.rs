//! Typed ids.
//!
//! Every entity id is a UUID wrapped in its own type, so a `ChatId` can never be passed where a
//! `ProjectId` is expected. Copilot generates ids in Rust as time-ordered UUID v7
//! (`ProjectId::generate()`). `UserId` is the exception: it wraps the id of a lib-rustauth user,
//! and Copilot never creates one.

use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Uuid);

        impl $name {
            /// Wraps an existing id (a stored row, a path parameter).
            pub const fn new(id: Uuid) -> Self {
                Self(id)
            }

            pub const fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(id: Uuid) -> Self {
                Self(id)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

/// Adds `generate()` to ids that Copilot creates.
macro_rules! generated_id {
    ($(#[$meta:meta])* $name:ident) => {
        typed_id!($(#[$meta])* $name);

        impl $name {
            /// A new time-ordered (v7) id.
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }
        }
    };
}

typed_id!(
    /// The id of a lib-rustauth user (`auth.users.id`).
    UserId
);
generated_id!(
    /// A provider connection.
    ProviderId
);
generated_id!(
    /// A model of a provider connection.
    ModelId
);
generated_id!(
    /// A project.
    ProjectId
);
generated_id!(
    /// A stored file (project file or chat attachment).
    FileId
);
generated_id!(
    /// A chat.
    ChatId
);
generated_id!(
    /// A message of a chat.
    MessageId
);
generated_id!(
    /// A tool call in an assistant message (Copilot's id, never the provider's).
    CallId
);
generated_id!(
    /// A registered MCP tool server.
    ToolServerId
);
generated_id!(
    /// A schedule.
    ScheduleId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_time_ordered_v7_ids() {
        let first = ProjectId::generate();
        let second = ProjectId::generate();
        assert_eq!(first.uuid().get_version_num(), 7);
        assert!(first < second);
    }

    #[test]
    fn round_trips_through_text() {
        let id = ChatId::generate();
        let parsed: ChatId = id.to_string().parse().expect("a printed id parses");
        assert_eq!(parsed, id);
        assert!("not-a-uuid".parse::<ChatId>().is_err());
    }

    #[test]
    fn wraps_a_rustauth_user_id() {
        let raw = Uuid::now_v7();
        let user = UserId::new(raw);
        assert_eq!(Uuid::from(user), raw);
        assert_eq!(UserId::from(raw), user);
    }
}
