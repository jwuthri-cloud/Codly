//! Careers and job offers: crediting agents for what their events say they did (rules in
//! `crate::career`), and the job offer posted in the office before a new consultant is hired.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{Inner, lock};
use crate::career::{self, Credit};
use crate::model::*;

/// What `claim` does with a new consultant.
pub(super) enum Hiring {
    /// Hire now: this applicant (None: the scheduler's own pick, no offer was posted).
    Now(Option<Applicant>),
    /// Its job offer is open: leave the task ready.
    Wait,
}

impl Inner {
    /// Credit agents for one persisted event (every event passes here).
    pub(super) fn track_career(&self, r: &EventRow) {
        let worker = || r.task_id.as_deref().and_then(|t| self.task(t).ok()).and_then(|t| t.assignee);
        let credits: Vec<(Option<Id>, Credit)> = match &r.kind {
            EventKind::Committed { .. } => vec![(r.agent_id.clone(), Credit::Commit)],
            EventKind::FilesChanged { paths, .. } => vec![(r.agent_id.clone(), Credit::Files(paths))],
            EventKind::Verified { ok, .. } => vec![(r.agent_id.clone().or_else(worker), Credit::Check { ok: *ok })],
            EventKind::Reviewed { verdict, .. } => {
                let w = worker();
                let reviewer = r.agent_id.clone().filter(|a| Some(a) != w.as_ref());
                vec![(w, Credit::Review { approved: verdict == "approve" }), (reviewer, Credit::Reviewed)]
            }
            EventKind::Merged { .. } => vec![(worker().or_else(|| r.agent_id.clone()), Credit::Merge)],
            _ => return,
        };
        for (id, c) in credits {
            let Some(id) = id else { continue };
            if self.agent(&id).is_ok_and(|a| a.role != Role::Lead) {
                let _ = self.update_agent(&id, |a| {
                    career::credit(&mut a.career, c, r.ts);
                });
            }
        }
    }

    /// How long a job offer stays up before the Lead's pick is hired (0: no offers, hire at once).
    pub(super) fn set_hiring_window(&self, d: Duration) {
        self.hiring_window_ms.store(d.as_millis() as u64, Ordering::Relaxed);
    }

    /// For `claim`: post a job offer for this task's new consultant, wait while it is open, and
    /// hand over the applicant once someone is hired. `name`/`color` are the scheduler's pick,
    /// the first applicant.
    pub(super) fn hiring(self: &Arc<Self>, ws: &str, task: &Task, provider: Provider, model: Option<String>, name: &str, color: i64) -> Hiring {
        let window = self.hiring_window_ms.load(Ordering::Relaxed);
        if window == 0 {
            return Hiring::Now(None);
        }
        let mut offers = lock(&self.offers);
        if let Some(o) = offers.get(&task.id) {
            return match o.hired {
                None => Hiring::Wait,
                Some(i) => {
                    let a = o.applicants.get(i as usize).cloned();
                    offers.remove(&task.id);
                    Hiring::Now(a)
                }
            };
        }
        let agents = self.store.agents(ws).unwrap_or_default();
        let mut taken: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
        taken.extend(offers.values().flat_map(|o| o.applicants.iter().map(|a| a.name.clone())));
        taken.push(name.to_string());
        let mut colors: Vec<i64> = agents.iter().map(|a| a.color).collect();
        colors.push(color);
        let seed = task.id.bytes().fold(now_ms() as u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
        // The scheduler's pick comes first (the Lead's pick); two more answer the same offer.
        let mut applicants = career::applicants(3, seed, &taken, &colors);
        applicants[0].name = name.to_string();
        applicants[0].color = color;
        let now = now_ms();
        let offer = JobOffer {
            id: new_id(),
            task_id: task.id.clone(),
            title: task.consultant.as_ref().and_then(|c| c.title.clone()),
            provider,
            model,
            applicants,
            posted_at: now,
            closes_at: now + window as i64,
            hired: None,
            by_you: false,
        };
        offers.insert(task.id.clone(), offer.clone());
        drop(offers);
        let _ = self.emit(Some(ws), super::row(None, Some(&task.id), None, EventKind::JobOffer { offer: offer.clone() }));
        let me = self.clone();
        let ws = ws.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(window)).await;
            me.close_offer(&ws, &offer.id, None);
        });
        Hiring::Wait
    }

    /// Close an open offer: `pick` is your choice; None hires the Lead's pick (the first).
    pub(super) fn close_offer(&self, ws: &str, offer_id: &str, pick: Option<usize>) -> bool {
        let closed = {
            let mut offers = lock(&self.offers);
            let Some(o) = offers.values_mut().find(|o| o.id == offer_id && o.hired.is_none()) else { return false };
            let i = pick.filter(|i| *i < o.applicants.len()).unwrap_or(0);
            o.hired = Some(i as i64);
            o.by_you = pick.is_some();
            o.clone()
        };
        let _ = self.emit(Some(ws), super::row(None, Some(&closed.task_id), None, EventKind::JobOffer { offer: closed.clone() }));
        self.kick();
        true
    }
}
