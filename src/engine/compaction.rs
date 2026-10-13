//! Shared compaction policy and session mutation; frontends own the notices.
use crate::{config::Config, context::ContextFiles, provider::AnyClient, session::Session};

pub(crate) fn reserve(session: &Session, cfg: &Config, context: &ContextFiles) -> u64 {
    let reserve = cfg.resolve_reserve_tokens(&session.model, &crate::config::quick_models_map(cfg));
    #[cfg(feature = "memory")]
    {
        crate::extras::memory::effective_reserve(reserve, context.memory.as_deref())
    }
    #[cfg(not(feature = "memory"))]
    {
        let _ = context;
        reserve
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    BelowBudget,
    NothingToCompact,
    Summarize { cut: usize },
}

pub(crate) fn plan(session: &Session, keep: u64, reserve: u64, auto: bool) -> Plan {
    if auto && session.effective_context_tokens() <= session.context_window.saturating_sub(reserve)
    {
        return Plan::BelowBudget;
    }
    match Session::select_compaction_cut(&session.messages, keep) {
        0 => Plan::NothingToCompact,
        cut => Plan::Summarize { cut },
    }
}

pub(crate) async fn apply(
    session: &mut Session,
    client: &AnyClient,
    cut: usize,
    instructions: Option<&str>,
) -> anyhow::Result<u64> {
    let messages = &session.messages[..cut];
    let summary = client
        .compress_messages(
            &session.model,
            messages,
            session.compactions.last().map(|c| c.summary.as_str()),
            instructions,
        )
        .await?;
    let tokens_before = messages.iter().map(|m| m.estimated_tokens).sum();
    #[cfg(feature = "memory")]
    crate::extras::memory::flush_compaction_summary(
        &crate::extras::memory::Mem::open(),
        &summary,
        Some(cut),
    );
    session.compress(summary, cut, tokens_before);
    Ok(tokens_before)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_compaction_bypasses_budget_and_respects_keep_recent() {
        let mut session = Session::new("anthropic", "model", 200_000, "");
        for _ in 0..4 {
            session.add_message(crate::session::MessageRole::User, &"question ".repeat(100));
            session.add_message(
                crate::session::MessageRole::Assistant,
                &"answer ".repeat(100),
            );
        }
        assert_eq!(plan(&session, 100, 1000, true), Plan::BelowBudget);
        let Plan::Summarize { cut } = plan(&session, 100, 1000, false) else {
            panic!("manual compaction should run")
        };
        assert!(cut > 0 && cut < session.messages.len());
        assert!(
            session.messages[cut..]
                .iter()
                .map(|m| m.estimated_tokens)
                .sum::<u64>()
                >= 100
        );
        assert_eq!(
            plan(&session, u64::MAX, 1000, false),
            Plan::NothingToCompact
        );
    }
}
