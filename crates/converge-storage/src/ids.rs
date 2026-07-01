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

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

// One per entity: group, project, decision (graph node), user, agent.
id!(GroupId);
id!(ProjectId);
id!(DecisionId);
id!(UserId);
id!(AgentId);
