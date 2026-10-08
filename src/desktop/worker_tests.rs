use super::*;
use crate::session::MessageRole;

#[test]
fn a_conversation_is_titled_by_its_name_or_first_message() {
    let mut session = Session::new("p", "m", 0, "");
    assert_eq!(title(&session), "New conversation");
    session.add_message(MessageRole::User, "\n  fix the build  \nthen test");
    assert_eq!(title(&session), "fix the build");
    session.name = "release".into();
    assert_eq!(title(&session), "release");
}
