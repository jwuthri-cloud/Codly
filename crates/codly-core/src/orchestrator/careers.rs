//! Careers and job offers: crediting agents for what their events say they did (rules in
//! `crate::career`), the job offer posted in the office before a new consultant is hired, and
//! ghosts: workers let go, who keep their career and can be brought back, and Build/Buy: the
//! XP they earned buys furniture for the office.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Result, bail};

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

/// What one event credits, and to whom (`worker`: the assignee of the event's task).
fn credits_of<'a>(kind: &'a EventKind, agent: Option<&Id>, worker: Option<&Id>) -> Vec<(Option<Id>, Credit<'a>)> {
    let agent = agent.cloned();
    let worker = worker.cloned();
    match kind {
        EventKind::Committed { .. } => vec![(agent, Credit::Commit)],
        EventKind::FilesChanged { paths, .. } => vec![(agent, Credit::Files(paths))],
        EventKind::Verified { ok, .. } => vec![(agent.or(worker), Credit::Check { ok: *ok })],
        EventKind::Reviewed { verdict, .. } => {
            let reviewer = agent.filter(|a| Some(a) != worker.as_ref());
            vec![(worker, Credit::Review { approved: verdict == "approve" }), (reviewer, Credit::Reviewed)]
        }
        EventKind::Merged { .. } => vec![(worker.or(agent), Credit::Merge)],
        _ => vec![],
    }
}

impl Inner {
    /// Credit agents for one persisted event (every event passes here).
    pub(super) fn track_career(&self, r: &EventRow) {
        if !matches!(r.kind, EventKind::Committed { .. } | EventKind::FilesChanged { .. } | EventKind::Verified { .. } | EventKind::Reviewed { .. } | EventKind::Merged { .. }) {
            return;
        }
        let worker = r.task_id.as_deref().and_then(|t| self.task(t).ok()).and_then(|t| t.assignee);
        for (id, c) in credits_of(&r.kind, r.agent_id.as_ref(), worker.as_ref()) {
            let Some(id) = id else { continue };
            if self.agent(&id).is_ok_and(|a| a.role != Role::Lead) {
                let _ = self.update_agent(&id, |a| {
                    career::credit(&mut a.career, c, r.ts);
                });
            }
        }
        if let (EventKind::Reviewed { .. }, Some(reviewer), Some(author)) = (&r.kind, r.agent_id.as_ref(), worker.as_ref())
            && reviewer != author
        {
            self.mentorship(reviewer, author, r.ts);
        }
    }

    /// A review makes reviewer and author friends; a Senior or Staff reviewing an Intern or
    /// Junior mentors it (bonus XP, and the first time, its mentor).
    fn mentorship(&self, reviewer: &Id, author: &Id, now: i64) {
        let (Ok(rv), Ok(au)) = (self.agent(reviewer), self.agent(author)) else { return };
        if rv.role == Role::Lead || au.role == Role::Lead {
            return;
        }
        let mentored = career::mentors(rv.career.rank, au.career.rank);
        let first = mentored && au.career.mentor.is_none();
        let _ = self.update_agent(author, |a| {
            career::befriend(&mut a.career, reviewer);
            if mentored {
                career::credit(&mut a.career, Credit::Mentored, now);
            }
            if first {
                a.career.mentor = Some(reviewer.clone());
                career::note(&mut a.career, now, format!("Mentored by {}", rv.name));
            }
        });
        let _ = self.update_agent(reviewer, |a| {
            career::befriend(&mut a.career, author);
            if first {
                career::note(&mut a.career, now, format!("Took {} under its wing", au.name));
            }
        });
    }

    /// What the office did since `since`, from its events (same XP rules as careers).
    pub(super) fn digest(&self, ws: &str, since: i64) -> Result<Digest> {
        let tasks = self.store.tasks(ws)?;
        // Consultants let go since then did that work too.
        let mut agents = self.store.agents(ws)?;
        agents.extend(self.store.archived_agents(ws)?);
        let lead = |id: &Id| agents.iter().any(|a| &a.id == id && a.role == Role::Lead);
        let mut by: std::collections::BTreeMap<Id, Career> = Default::default();
        for r in self.store.work_since(ws, since)? {
            let worker = r.task_id.as_deref().and_then(|t| tasks.iter().find(|x| x.id == t)).and_then(|t| t.assignee.as_ref());
            for (id, c) in credits_of(&r.kind, r.agent_id.as_ref(), worker) {
                if let Some(id) = id.filter(|i| !lead(i)) {
                    career::credit(by.entry(id).or_default(), c, r.ts);
                }
            }
        }
        let mut reports: Vec<DayReport> = by.into_iter().map(|(agent_id, c)| DayReport { agent_id, stats: c.stats, xp: c.xp }).collect();
        reports.sort_by(|a, b| b.xp.cmp(&a.xp).then(a.agent_id.cmp(&b.agent_id)));
        let mut milestones: Vec<(Id, Milestone)> = agents
            .iter()
            .filter(|a| a.role != Role::Lead)
            .flat_map(|a| a.career.milestones.iter().filter(|m| m.at >= since).map(|m| (a.id.clone(), m.clone())))
            .collect();
        milestones.sort_by_key(|(_, m)| m.at);
        let done = tasks.iter().filter(|t| t.status == TaskStatus::Done && t.updated_at >= since).map(|t| t.title.clone()).collect();
        Ok(Digest { since, reports, milestones, done })
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

    /// Workers let go in this repo (consultants after their task), newest first. Team members
    /// come back through Settings and Leads through their thread, so they are not ghosts.
    pub(super) fn ghosts(&self, ws: &str) -> Result<Vec<Agent>> {
        Ok(self.store.archived_agents(ws)?.into_iter().filter(|a| a.role == Role::Worker && a.member.is_none()).collect())
    }

    /// Bring a ghost back as a regular worker: the scheduler reuses it for tasks of its
    /// provider and you can assign it work. Its career, memory and look are the same.
    pub(super) fn bring_back(&self, ws: &str, id: &str) -> Result<Agent> {
        let Some(g) = self.ghosts(ws)?.into_iter().find(|a| a.id == id) else { bail!("no such ghost in this repo") };
        if self.store.agents(ws)?.iter().any(|a| a.name.eq_ignore_ascii_case(&g.name)) {
            bail!("someone called {} already works here", g.name);
        }
        let now = now_ms();
        self.update_agent(id, |a| {
            a.archived = false;
            a.consultant = false;
            a.status = AgentStatus::Idle;
            a.task_id = None;
            a.activity = None;
            a.activity_label = None;
            career::note(&mut a.career, now, "Came back".into());
        })
    }

    // (`shop:{ws}`, the first fixed-spot catalog, is no longer read: those buys are refunded.)
    pub(super) fn shop(&self, ws: &str) -> Result<Shop> {
        let placed: Vec<Placed> = self.store.kv_get(&format!("decor:{ws}"))?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        let placed: Vec<Placed> = placed.into_iter().filter(|p| career::price(&p.item).is_some()).collect();
        let earned = self.store.agents(ws)?.iter().chain(self.store.archived_agents(ws)?.iter()).map(|a| a.career.xp).sum();
        let spent = placed.iter().filter_map(|p| career::price(&p.item)).sum();
        Ok(Shop { earned, spent, placed })
    }

    /// Buy a piece of furniture and put it down, or move one already bought (same `uid`).
    pub(super) fn place(&self, ws: &str, piece: Placed) -> Result<Shop> {
        let mut shop = self.shop(ws)?;
        if let Some(p) = shop.placed.iter_mut().find(|p| p.uid == piece.uid) {
            if p.item != piece.item {
                bail!("that piece is a {}", p.item);
            }
            *p = piece;
        } else {
            let Some(price) = career::price(&piece.item) else { bail!("no such item") };
            if shop.earned - shop.spent < price {
                bail!("not enough XP yet ({} of {price})", shop.earned - shop.spent);
            }
            shop.placed.push(piece);
        }
        self.save_decor(ws, &shop.placed)
    }

    /// Sell a piece back, for the full price.
    pub(super) fn sell(&self, ws: &str, uid: &str) -> Result<Shop> {
        let mut shop = self.shop(ws)?;
        let n = shop.placed.len();
        shop.placed.retain(|p| p.uid != uid);
        if shop.placed.len() == n {
            bail!("not in the office");
        }
        self.save_decor(ws, &shop.placed)
    }

    fn save_decor(&self, ws: &str, placed: &[Placed]) -> Result<Shop> {
        self.store.kv_set(&format!("decor:{ws}"), &serde_json::to_string(placed)?)?;
        self.shop(ws)
    }
}
