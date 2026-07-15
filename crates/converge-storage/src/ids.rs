//! Type-safe id newtypes over [`Ulid`] — one per entity, so a `ProjectId`
//! can't be passed where a `DecisionId` is expected. Stored as `uuid` in
//! Postgres (same 128 bits); the storage layer converts at the boundary.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Ulid);

        impl $name {
            /// Mint a fresh, time-ordered id.
            #[expect(
                clippy::new_without_default,
                reason = "a fresh unique id is not a default value"
            )]
            pub fn new() -> Self {
                Self(Ulid::new())
            }
            pub fn ulid(self) -> Ulid {
                self.0
            }
        }

        impl From<Ulid> for $name {
            fn from(u: Ulid) -> Self {
                Self(u)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse::<Ulid>().map(Self)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

// One per entity: group, project, decision (graph node), user, agent,
// bearer token, session (evidence container), message, signal.
id!(GroupId);
id!(ProjectId);
id!(DecisionId);
id!(UserId);
id!(AgentId);
id!(TokenId);
id!(SessionId);
id!(MessageId);
id!(SignalId);
