//! The wallet (🪙): one per player, for every repo, separate from XP (docs/design/PROGRESSION.md
//! §3). Coins come in from real work (every agent's XP, in every repo, let go ones too) and from
//! grants; they go out on lots, building and furniture, each purchase and refund a line in the
//! ledger (`wallet:ledger`), so the balance is always derived, never stored. Developer cheats are
//! grants with reason `CHEAT` and flag the wallet for good. Build/Buy is here too: the open
//! repo's furniture, which lives on its lot (lots.rs prices and stores it).

use std::sync::{Mutex, MutexGuard};

use anyhow::{Result, bail};

use crate::model::*;
use crate::store::Store;

/// The ledger of grants, a JSON list of `Grant`.
const GRANTS: &str = "wallet:grants";
/// What was paid and given back, a JSON list of `Spend`.
const LEDGER: &str = "wallet:ledger";
/// Set by the first cheat and never cleared: stored, not derived from a ledger a reset could empty.
const CHEATED: &str = "wallet:cheated";
/// The reason the developer panel's grants carry.
pub const CHEAT: &str = "cheat";

/// Held from reading what you own to paying for the change, so two purchases can't both spend
/// the same coins.
pub(super) fn money() -> MutexGuard<'static, ()> {
    static MONEY: Mutex<()> = Mutex::new(());
    MONEY.lock().unwrap_or_else(|e| e.into_inner())
}

fn grants(store: &Store) -> Result<Vec<Grant>> {
    Ok(store.kv_get(GRANTS)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

fn ledger(store: &Store) -> Result<Vec<Spend>> {
    Ok(store.kv_get(LEDGER)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

/// The kv write that adds a line to the ledger (for a caller's own transaction).
pub(super) fn ledger_line(store: &Store, amount: i64, what: &str) -> Result<(String, Option<String>)> {
    let mut all = ledger(store)?;
    all.push(Spend { amount, what: what.into(), at: now_ms() });
    Ok((LEDGER.into(), Some(serde_json::to_string(&all)?)))
}

/// The player's wallet: the one place coins are counted.
pub fn wallet(store: &Store) -> Result<Wallet> {
    // Every repo on its plot first: furniture bought before lots joins the ledger there.
    super::lots::slots(store)?;
    let earned = store.every_agent()?.iter().map(|a| a.career.xp).sum();
    let granted = grants(store)?.iter().map(|g| g.amount).sum();
    let spent = ledger(store)?.iter().map(|s| s.amount).sum();
    let cheated = store.kv_get(CHEATED)?.is_some();
    Ok(Wallet { earned, granted, spent, balance: earned + granted - spent, cheated })
}

/// Pay `cost` (negative: give it back) for `what`, with the writes it buys, in one transaction.
/// Refused when the wallet can't pay. The caller holds `money()`.
pub(super) fn pay(store: &Store, cost: i64, what: &str, mut writes: Vec<(String, Option<String>)>) -> Result<Wallet> {
    if cost > 0 {
        let left = wallet(store)?.balance;
        if left < cost {
            bail!("not enough coins yet ({left} of {cost})");
        }
    }
    if cost != 0 {
        writes.push(ledger_line(store, cost, what)?);
    }
    store.kv_write(&writes)?;
    wallet(store)
}

/// A cheat was used: flag the wallet, for good.
pub(super) fn mark_cheated(store: &Store) -> Result<()> {
    if store.kv_get(CHEATED)?.is_none() {
        store.kv_set(CHEATED, &now_ms().to_string())?;
    }
    Ok(())
}

/// Give coins outright. A `CHEAT` grant flags the wallet before it lands.
pub(super) fn grant(store: &Store, amount: i64, reason: &str) -> Result<Wallet> {
    if reason == CHEAT {
        mark_cheated(store)?;
    }
    let mut all = grants(store)?;
    all.push(Grant { amount, reason: reason.into(), at: now_ms() });
    store.kv_set(GRANTS, &serde_json::to_string(&all)?)?;
    wallet(store)
}

/// Build/Buy in one repo: the wallet and the furniture bought for this office (what came with
/// the lot and the office's own stations are drawn from the lot; the games the shop sells, like
/// the foosball, are both: bought, and standing in the lot where they play).
pub(super) fn shop(store: &Store, ws: &str) -> Result<Shop> {
    let lot = super::lots::lot_of(store, ws)?;
    let gift: std::collections::HashSet<&String> = lot.gift.iter().collect();
    let bought = |p: &Placed| !crate::lot::builtin(&p.item) || crate::career::sold_builtin(&p.item);
    let placed = lot.furniture.iter().filter(|p| !gift.contains(&p.uid) && bought(p)).cloned().collect();
    Ok(Shop { wallet: wallet(store)?, placed })
}

/// Buy a piece of furniture and put it down, or move one already there (same `uid`, free; one
/// in storage comes out).
pub(super) fn place(store: &Store, ws: &str, piece: Placed) -> Result<Shop> {
    super::lots::change(store, ws, "furniture", |lot| {
        if let Some(p) = lot.furniture.iter_mut().find(|p| p.uid == piece.uid) {
            if p.item != piece.item {
                bail!("that piece is a {}", p.item);
            }
            *p = piece;
        } else if let Some(i) = lot.storage.iter().position(|p| p.uid == piece.uid) {
            if lot.storage[i].item != piece.item {
                bail!("that piece is a {}", lot.storage[i].item);
            }
            lot.storage.remove(i);
            lot.furniture.push(piece);
        } else {
            if crate::career::price(&piece.item).is_none() {
                bail!("no such item");
            }
            // Earned pieces (furniture.json `needs`): a rank on the crew, or a kitchen here.
            let crew: Vec<Rank> = store.agents(ws)?.iter().map(|a| a.career.rank).collect();
            if let Some(why) = crate::career::locked(&piece.item, &crew, lot.furniture.iter().map(|p| p.item.as_str())) {
                bail!("{why}");
            }
            lot.furniture.push(piece);
        }
        Ok(())
    })?;
    shop(store, ws)
}

/// Sell a piece back, for the full price (a gift for nothing).
pub(super) fn sell(store: &Store, ws: &str, uid: &str) -> Result<Shop> {
    super::lots::change(store, ws, "furniture", |lot| {
        let n = lot.furniture.len() + lot.storage.len();
        lot.furniture.retain(|p| p.uid != uid);
        lot.storage.retain(|p| p.uid != uid);
        if lot.furniture.len() + lot.storage.len() == n {
            bail!("not in the office");
        }
        Ok(())
    })?;
    shop(store, ws)
}

/// The developer panel's "Reset this office": its lot goes back to a gift Garage, and what stood
/// there sells back at the usual rates.
pub(super) fn reset_office(store: &Store, ws: &str) -> Result<Shop> {
    super::lots::reset(store, ws)?;
    shop(store, ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::career;

    fn workspace(store: &Store, name: &str) -> String {
        let id = new_id();
        let ws = Workspace {
            id: id.clone(),
            repo_path: format!("/tmp/{name}"),
            name: name.into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&ws).unwrap();
        id
    }

    fn hire(store: &Store, ws: &str, xp: i64, archived: bool) {
        let id = new_id();
        let agent = Agent {
            id: id.clone(),
            name: format!("A{}", &id[..4]),
            provider: Provider::Claude,
            model: None,
            role: Role::Worker,
            status: AgentStatus::Idle,
            activity: None,
            activity_label: None,
            task_id: None,
            run_id: None,
            session_id: None,
            worktree: None,
            billing: Billing::Subscription,
            account: None,
            color: 0,
            look_seed: id.clone(),
            thread: None,
            member: None,
            title: None,
            consultant: false,
            kept: false,
            away_until: None,
            tired_since: None,
            archived,
            usage: Usage::default(),
            career: Career { xp, ..Default::default() },
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, workspace_id: None, kind: EventKind::AgentChanged { agent } };
        store.append_event(Some(ws), &row).unwrap();
    }

    fn chair(uid: &str) -> Placed {
        Placed { uid: uid.into(), item: "kenney.chair".into(), x: 1.0, y: 0.0, z: 1.0, rot: 0.0 }
    }

    #[test]
    fn the_wallet_counts_every_repo() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = (workspace(&store, "a"), workspace(&store, "b"));
        hire(&store, &a, 100, false);
        hire(&store, &b, 50, true); // let go, in another repo: still earned
        let price = career::price("kenney.chair").unwrap();
        place(&store, &b, chair("c1")).unwrap();
        let w = wallet(&store).unwrap();
        assert_eq!((w.earned, w.granted, w.spent, w.balance, w.cheated), (150, 0, price, 150 - price, false));
        // Every repo sees the same wallet; each its own furniture.
        let (sa, sb) = (shop(&store, &a).unwrap(), shop(&store, &b).unwrap());
        assert_eq!(sa.wallet, sb.wallet);
        assert_eq!((sa.placed.len(), sb.placed.len()), (0, 1));
    }

    #[test]
    fn a_cheat_flags_the_wallet_for_good() {
        let store = Store::open_in_memory().unwrap();
        assert!(!grant(&store, 10, "gift").unwrap().cheated, "only cheats flag it");
        let w = grant(&store, 1000, CHEAT).unwrap();
        assert_eq!((w.granted, w.balance, w.cheated), (1010, 1010, true));
        store.kv_set(GRANTS, "[]").unwrap(); // the ledger emptied: the flag stays
        let w = wallet(&store).unwrap();
        assert_eq!((w.granted, w.cheated), (0, true));
    }

    #[test]
    fn buying_beyond_the_balance_fails() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = (workspace(&store, "a"), workspace(&store, "b"));
        let price = career::price("kenney.chair").unwrap();
        hire(&store, &a, price + price / 2, false);
        place(&store, &a, chair("c1")).unwrap();
        // The other repo spends from the same wallet.
        let err = place(&store, &b, chair("c2")).unwrap_err().to_string();
        assert!(err.contains("not enough coins"), "{err}");
        // Moving is free; selling gives the price back; a cheat pays for it.
        let s = place(&store, &a, Placed { x: 3.0, ..chair("c1") }).unwrap();
        assert_eq!((s.wallet.spent, s.placed[0].x), (price, 3.0));
        sell(&store, &a, "c1").unwrap();
        place(&store, &b, chair("c2")).unwrap();
        assert!(place(&store, &b, chair("c3")).is_err());
        grant(&store, price, CHEAT).unwrap();
        let s = place(&store, &b, chair("c3")).unwrap();
        assert_eq!(s.wallet.balance, price / 2);
        // Resetting an office sells its furniture back (all of it: furniture sells at 100%).
        let s = reset_office(&store, &b).unwrap();
        assert!(s.placed.is_empty(), "{s:?}");
        assert_eq!((s.wallet.spent, s.wallet.balance), (0, 2 * price + price / 2));
        // Every purchase and refund is a line in the ledger; the balance is their sum.
        let lines = ledger(&store).unwrap();
        assert_eq!(lines.iter().map(|l| l.amount).collect::<Vec<_>>(), [price, -price, price, price, -2 * price]);
    }

    #[test]
    fn furniture_in_storage_comes_out_free_and_sells() {
        let store = Store::open_in_memory().unwrap();
        let a = workspace(&store, "a");
        grant(&store, 100, "test").unwrap();
        // Put a bought chair away (as a move of the office would), then take it out again.
        place(&store, &a, chair("c1")).unwrap();
        super::super::lots::change(&store, &a, "test", |lot| {
            let p = lot.furniture.pop().unwrap();
            lot.storage.push(p);
            Ok(())
        })
        .unwrap();
        assert!(shop(&store, &a).unwrap().placed.is_empty());
        let s = place(&store, &a, Placed { x: 4.0, ..chair("c1") }).unwrap();
        assert_eq!((s.placed.len(), s.wallet.spent), (1, 25), "out of storage: free");
        assert!(place(&store, &a, Placed { item: "kenney.loungeSofa".into(), ..chair("c1") }).is_err());
        assert_eq!(sell(&store, &a, "c1").unwrap().wallet.spent, 0);
    }
}
