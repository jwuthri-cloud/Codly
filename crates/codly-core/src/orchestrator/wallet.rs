//! The wallet (🪙): one per player, for every repo, separate from XP (docs/design/PROGRESSION.md
//! §3). Coins come in from real work (every agent's XP, in every repo, let go ones too) and from
//! grants; they go out on what you own (every repo's furniture). Developer cheats are grants with
//! reason `CHEAT` and flag the wallet for good. Build/Buy is here too: an office's furniture
//! (`decor:{ws}`), bought, moved and sold back against the wallet.

use anyhow::{Result, bail};

use crate::career;
use crate::model::*;
use crate::store::Store;

/// The ledger of grants, a JSON list of `Grant`.
const GRANTS: &str = "wallet:grants";
/// Set by the first cheat and never cleared: stored, not derived from a ledger a reset could empty.
const CHEATED: &str = "wallet:cheated";
/// The reason the developer panel's grants carry.
pub const CHEAT: &str = "cheat";

fn decor_key(ws: &str) -> String {
    format!("decor:{ws}")
}

/// Catalog pieces only. (`shop:{ws}`, the first fixed-spot catalog, is no longer read: those
/// buys are refunded.)
fn pieces(json: &str) -> Vec<Placed> {
    serde_json::from_str::<Vec<Placed>>(json).unwrap_or_default().into_iter().filter(|p| career::price(&p.item).is_some()).collect()
}

/// The furniture in one repo's office.
pub(super) fn placed(store: &Store, ws: &str) -> Result<Vec<Placed>> {
    Ok(store.kv_get(&decor_key(ws))?.map(|v| pieces(&v)).unwrap_or_default())
}

fn grants(store: &Store) -> Result<Vec<Grant>> {
    Ok(store.kv_get(GRANTS)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

/// What you own, at its price, one line per kind; the wallet's `spent` is their sum. Lots and
/// buildings (PROGRESSION.md §4) add their lines here.
fn spending(store: &Store) -> Result<Vec<i64>> {
    let furniture = store.kv_prefixed("decor:")?.iter().flat_map(|(_, v)| pieces(v)).filter_map(|p| career::price(&p.item)).sum();
    Ok(vec![furniture])
}

/// The player's wallet: the one place coins are counted.
pub fn wallet(store: &Store) -> Result<Wallet> {
    let earned = store.every_agent()?.iter().map(|a| a.career.xp).sum();
    let granted = grants(store)?.iter().map(|g| g.amount).sum();
    let spent = spending(store)?.iter().sum();
    let cheated = store.kv_get(CHEATED)?.is_some();
    Ok(Wallet { earned, granted, spent, balance: earned + granted - spent, cheated })
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

/// Build/Buy in one repo: the wallet and this office's furniture.
pub(super) fn shop(store: &Store, ws: &str) -> Result<Shop> {
    Ok(Shop { wallet: wallet(store)?, placed: placed(store, ws)? })
}

/// Buy a piece of furniture and put it down, or move one already bought (same `uid`, free).
pub(super) fn place(store: &Store, ws: &str, piece: Placed) -> Result<Shop> {
    let mut all = placed(store, ws)?;
    if let Some(p) = all.iter_mut().find(|p| p.uid == piece.uid) {
        if p.item != piece.item {
            bail!("that piece is a {}", p.item);
        }
        *p = piece;
    } else {
        let Some(price) = career::price(&piece.item) else { bail!("no such item") };
        let left = wallet(store)?.balance;
        if left < price {
            bail!("not enough coins yet ({left} of {price})");
        }
        all.push(piece);
    }
    save(store, ws, &all)
}

/// Sell a piece back, for the full price.
pub(super) fn sell(store: &Store, ws: &str, uid: &str) -> Result<Shop> {
    let mut all = placed(store, ws)?;
    let n = all.len();
    all.retain(|p| p.uid != uid);
    if all.len() == n {
        bail!("not in the office");
    }
    save(store, ws, &all)
}

/// The developer panel's "Reset this office": its furniture goes (and what it cost comes back).
pub(super) fn reset_office(store: &Store, ws: &str) -> Result<Shop> {
    // Phase 2 (PROGRESSION.md §4): also put the lot back to the garage (`lot:{ws}`) here.
    save(store, ws, &[])
}

fn save(store: &Store, ws: &str, placed: &[Placed]) -> Result<Shop> {
    store.kv_set(&decor_key(ws), &serde_json::to_string(placed)?)?;
    shop(store, ws)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            archived,
            usage: Usage::default(),
            career: Career { xp, ..Default::default() },
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind: EventKind::AgentChanged { agent } };
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
        // Resetting an office empties it, and what it cost comes back.
        let s = reset_office(&store, &b).unwrap();
        assert!(s.placed.is_empty(), "{s:?}");
        assert_eq!((s.wallet.spent, s.wallet.balance), (0, 2 * price + price / 2));
    }
}
