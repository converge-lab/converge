//! The hook wire format Claude Code defined and Codex CLI adopted
//! verbatim: a `hookSpecificOutput` envelope tagged with its event name,
//! beside a top-level `systemMessage`. Codex ships a JSON Schema for it
//! (`SessionStartHookSpecificOutputWire` and friends) that matches
//! Claude's field for field, so both harnesses answer through here.
//!
//! Cursor's contract is the same *shape* in snake_case. That is
//! deliberately not shared: two vendors who agree today are two vendors
//! who can diverge tomorrow, and a single function serving both is how
//! you end up unable to follow either.

use serde_json::{Value, json};

use super::Response;

pub fn emit(response: Response) -> Option<Value> {
    Some(match response {
        Response::Inject {
            context, system, ..
        } => json!({
            "systemMessage": system,
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            },
        }),
        Response::Ctx { tool_input } => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "updatedInput": tool_input,
            },
        }),
        Response::Notice { system } => json!({ "systemMessage": system }),
        Response::Marked {
            system: Some(system),
            ..
        } => json!({ "systemMessage": system }),
        Response::Marked { system: None, .. } | Response::Silent => return None,
    })
}
