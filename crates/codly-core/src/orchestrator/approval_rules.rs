//! "Always allow in this repo": rules you save from an approval card, kept in Codly's store
//! (table `approval_rule`) and followed by the broker for every agent of that repo. What a rule
//! may cover is decided in `approvals::rule_offer`; this file saves, lists and removes rules,
//! and answers the requests that wait when a rule is saved.
//!
//! - A rule is always built from a request that waits for you: its text is what the broker
//!   recorded for that request (`Store::set_rule_offer`), never text sent by the UI.
//! - Saving a rule answers every waiting request of the repo that offers the same rule: the one
//!   on the card and the same command asked by other agents.

use anyhow::{anyhow, bail};
use serde_json::json;

use super::approvals::{self, PolicyCtx, Verdict};
use super::{Core, Inner, lock};
use crate::model::*;

/// Held while the broker reads the rules and records what a request can be saved as, and
/// while a new rule is saved and the waiting requests are read: a request asked at that
/// moment is either allowed by the new rule or answered with it, never left waiting.
static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How a rule reads on a card and in the decision log.
fn describe(rule: &ApprovalRule) -> String {
    match rule.kind {
        ApprovalRuleKind::Exact => format!("`{}`", rule.command),
        ApprovalRuleKind::Prefix => format!("commands that start with `{}`", rule.command),
    }
}

impl Inner {
    /// The broker's last step: a request the rules would ask you about is allowed when a rule
    /// you saved for the repo covers it. A request that still asks keeps what it can be saved
    /// as, for its card.
    pub(super) fn with_saved_rules(&self, ws: Option<&str>, a: &Approval, ctx: &PolicyCtx, verdict: Verdict) -> Verdict {
        let Some(ws) = ws else { return verdict };
        if !matches!(verdict, Verdict::Ask { .. }) {
            return verdict;
        }
        let offer = approvals::rule_offer(a, ctx);
        if offer.is_empty() {
            return verdict;
        }
        let _g = lock(&SAVING);
        match self.store.approval_rules(ws) {
            Ok(rules) => {
                if let Some(rule) = rules.iter().find(|r| offer.offers(r)) {
                    return Verdict::Allow { choice: ApprovalChoice::AllowOnce, rule: format!("your rule for this repo: {}", describe(rule)) };
                }
            }
            // Asking is the safe answer when the rules cannot be read.
            Err(e) => tracing::warn!("saved approval rules not read, asking instead: {e}"),
        }
        if let Err(e) = self.store.set_rule_offer(&a.id, &offer) {
            tracing::warn!("the rules this request can be saved as were not recorded: {e}");
        }
        verdict
    }

    fn add_approval_rule(&self, approval_id: &str, kind: ApprovalRuleKind) -> anyhow::Result<ApprovalRule> {
        let a = self.store.approval(approval_id)?.ok_or_else(|| anyhow!("unknown approval {approval_id}"))?;
        if a.status != ApprovalStatus::Pending {
            bail!("that approval was already answered");
        }
        let offer = self.store.rule_offer(approval_id)?;
        let Some(command) = offer.text(kind) else {
            bail!("Codly always asks about this kind of request, so it cannot be saved as a rule");
        };
        let ws = self.store.agent_workspace(&a.agent_id)?.ok_or_else(|| anyhow!("that agent belongs to no repository"))?;
        let (rule, covered) = {
            let _g = lock(&SAVING);
            let rule = self.store.add_approval_rule(&ws, kind, command)?;
            let waiting = self.store.pending_rule_offers(&ws)?;
            let covered: Vec<Id> = waiting.into_iter().filter(|(_, o)| o.offers(&rule)).map(|(id, _)| id).collect();
            (rule, covered)
        };
        self.decide_by(
            Some(&ws),
            None,
            "approval_policy",
            "human",
            format!("you saved a rule: always allow {} in this repo", describe(&rule)),
            json!({ "rule": rule, "approval_id": approval_id }),
        )?;
        // The request on the card, and the same one from other agents: answered together.
        for id in covered {
            let once = ApprovalDecision { choice: ApprovalChoice::AllowOnce, message: None, answers: None };
            // An agent that is gone has its request marked expired: the rule is saved all the same.
            if let Err(e) = self.resolve_approval(&id, once, "user") {
                tracing::debug!("a request the new rule covers was not answered: {e}");
            }
        }
        Ok(rule)
    }

    fn remove_approval_rule(&self, rule_id: &str) -> anyhow::Result<()> {
        let rule = self.store.remove_approval_rule(rule_id)?.ok_or_else(|| anyhow!("that rule was already removed"))?;
        self.decide_by(
            Some(&rule.workspace_id),
            None,
            "approval_policy",
            "human",
            format!("you removed a rule: Codly asks again about {}", describe(&rule)),
            json!({ "rule": rule }),
        )?;
        Ok(())
    }
}

impl Core {
    /// The rules a waiting request can be saved as (its card offers these, and nothing for
    /// what Codly always asks about).
    pub fn approval_rule_offer(&self, approval_id: &str) -> anyhow::Result<ApprovalRuleOffer> {
        self.inner.store.rule_offer(approval_id)
    }

    /// "Always allow in this repo", from a waiting request: saves the rule for the repo of the
    /// agent that asked, and answers every waiting request there that the rule covers.
    pub fn add_approval_rule(&self, approval_id: &str, kind: ApprovalRuleKind) -> anyhow::Result<ApprovalRule> {
        self.inner.add_approval_rule(approval_id, kind)
    }

    /// The open repo's saved rules, oldest first.
    pub fn approval_rules(&self) -> anyhow::Result<Vec<ApprovalRule>> {
        let ws = self.inner.require_ws()?;
        self.inner.store.approval_rules(&ws.id)
    }

    /// Remove a saved rule: Codly asks again from the next request on.
    pub fn remove_approval_rule(&self, rule_id: &str) -> anyhow::Result<()> {
        self.inner.remove_approval_rule(rule_id)
    }
}
