//! zerostack's `_meta` on ACP responses.

use agent_client_protocol::schema::v1::Meta;

/// What a client should show about a session it opened, as response
/// `_meta`: `{"zerostack": {"notices": [...]}}`, when there is any.
pub(super) fn notices(notices: Vec<String>) -> Option<Meta> {
    if notices.is_empty() {
        return None;
    }
    let mut meta = Meta::new();
    meta.insert(
        "zerostack".to_string(),
        serde_json::json!({ "notices": notices }),
    );
    Some(meta)
}

/// The conversation as zerostack keeps it, as prompt response `_meta`, for a
/// client of a process that does not save it: `{"zerostack": {"session":
/// {...}}}`.
pub(super) fn session(session: &crate::session::Session) -> Meta {
    let mut meta = Meta::new();
    meta.insert(
        "zerostack".to_string(),
        serde_json::json!({ "session": session }),
    );
    meta
}
