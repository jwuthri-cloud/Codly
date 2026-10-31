//! The player's lots (PROGRESSION.md §4): plots on the neighbourhood map (`lots:slots`, one list
//! per player), each plot's lot (`lot:slot:{id}`), and at most one repo's office on each. Build
//! mode's saves, Build/Buy's furniture, buying, selling and moving are priced by `crate::price`
//! and paid from the wallet (wallet.rs) in the same transaction as what they change.
//!
//! The first read moves what came before lots in: every repo gets a plot of its own, free, with
//! the lot it had (`lot:{ws}`, or the Studio or Garage it would have got), and the furniture
//! Build/Buy sold it (`decor:{ws}`) joins that lot and the wallet's ledger. After that, a newly
//! opened repo gets a small plot with the Garage, both gifts: every office can work from day one,
//! and gifts sell for nothing, so a free Garage can't be farmed for coins.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::{Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};

use super::{Inner, wallet};
use crate::lot::{self as design, kept_gifts};
use crate::model::*;
use crate::price;
use crate::store::Store;

const SLOTS: &str = "lots:slots";
/// The neighbourhood is this many plots wide; more rows are added as needed.
const COLS: i64 = 4;
/// The neighbourhood at first: 4 × 3 plots, row by row.
const SEED: [LotSizeName; 12] = {
    use LotSizeName::*;
    [Small, Small, Medium, Large, Small, Medium, Medium, Large, Small, Small, Medium, Large]
};

fn lot_key(slot: &str) -> String {
    format!("lot:slot:{slot}")
}

/// A plot as stored (`LotSlot` is it with what the map shows).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Row {
    id: String,
    x: i64,
    z: i64,
    size: LotSizeName,
    owned: bool,
    #[serde(default)]
    workspace: Option<Id>,
    /// The land came free.
    #[serde(default)]
    gift: bool,
}

fn dims(size: LotSizeName) -> (f64, f64) {
    let p = price::plot(size);
    (p.w, p.d)
}

fn json<T: Serialize>(v: &T) -> Result<Option<String>> {
    Ok(Some(serde_json::to_string(v)?))
}

fn read_rows(store: &Store) -> Result<Option<Vec<Row>>> {
    Ok(store.kv_get(SLOTS)?.and_then(|v| serde_json::from_str(&v).ok()))
}

fn seed() -> Vec<Row> {
    SEED.iter().enumerate().map(|(i, &size)| Row { id: format!("lot-{}", i + 1), x: i as i64 % COLS, z: i as i64 / COLS, size, owned: false, workspace: None, gift: false }).collect()
}

/// A plot of `size` that isn't anyone's (a new one at the end of the map if none is left).
fn free_plot(rows: &mut Vec<Row>, size: LotSizeName) -> usize {
    if let Some(i) = rows.iter().position(|r| !r.owned && r.size == size) {
        return i;
    }
    let n = rows.len() as i64;
    let mut k = n + 1;
    while rows.iter().any(|r| r.id == format!("lot-{k}")) {
        k += 1;
    }
    rows.push(Row { id: format!("lot-{k}"), x: n % COLS, z: n / COLS, size, owned: false, workspace: None, gift: false });
    rows.len() - 1
}

/// Catalog pieces only, as Build/Buy kept them.
fn decor_pieces(json: &str) -> Vec<Placed> {
    serde_json::from_str::<Vec<Placed>>(json).unwrap_or_default().into_iter().filter(|p| crate::career::price(&p.item).is_some()).collect()
}

fn inside(p: &Placed, w: f64, d: f64) -> bool {
    (0.0..=w).contains(&p.x) && (0.0..=d).contains(&p.z)
}

/// Every plot, with every repo on one (`wallet()` reads it first). The first read moves what
/// came before lots in (see the module doc); later, a repo opened since gets its plot here.
pub(super) fn slots(store: &Store) -> Result<Vec<Row>> {
    let offices = store.offices()?;
    let housed = |rows: &[Row]| offices.iter().all(|o| rows.iter().any(|r| r.workspace.as_deref() == Some(o.id.as_str())));
    if let Some(rows) = read_rows(store)?
        && housed(&rows)
    {
        return Ok(rows);
    }
    static ENSURE: Mutex<()> = Mutex::new(());
    let _g = ENSURE.lock().unwrap_or_else(|e| e.into_inner());
    let mut writes = vec![];
    let mut rows = match read_rows(store)? {
        Some(rows) if housed(&rows) => return Ok(rows),
        Some(rows) => rows,
        None => {
            // Before lots: each repo's lot (or the one it would have got), its furniture, paid.
            let mut rows = seed();
            for o in offices.iter().rev() {
                let decor = store.kv_get(&format!("decor:{}", o.id))?.map(|v| decor_pieces(&v)).unwrap_or_default();
                let mut lot = match store.kv_get(&format!("lot:{}", o.id))?.and_then(|v| serde_json::from_str::<Lot>(&v).ok()) {
                    Some(lot) => lot,
                    None => {
                        let existing = !decor.is_empty() || !store.agents(&o.id)?.is_empty() || !store.archived_agents(&o.id)?.is_empty() || !store.tasks(&o.id)?.is_empty();
                        design::first(existing, decor.clone())
                    }
                };
                let taken = design::ids(&lot);
                let (w, d) = (lot.size.w, lot.size.d);
                for p in decor.into_iter().filter(|p| !taken.contains(&p.uid)) {
                    if inside(&p, w, d) { lot.furniture.push(p) } else { lot.storage.push(p) }
                }
                let size = price::size_for(w, d).unwrap_or(LotSizeName::Large);
                let (pw, pd) = dims(size);
                let lot = design::fit(lot.clone(), pw, pd).unwrap_or(lot);
                let i = free_plot(&mut rows, size);
                (rows[i].owned, rows[i].workspace, rows[i].gift) = (true, Some(o.id.clone()), true);
                writes.push((lot_key(&rows[i].id), json(&lot)?));
                writes.push((format!("lot:{}", o.id), None));
            }
            // What Build/Buy sold, every repo's (gone ones too), is in the ledger from now on.
            let mut bought = 0;
            for (key, v) in store.kv_prefixed("decor:")? {
                bought += decor_pieces(&v).iter().filter_map(|p| crate::career::price(&p.item)).sum::<i64>();
                writes.push((key, None));
            }
            if bought > 0 {
                writes.push(wallet::ledger_line(store, bought, "furniture bought before lots")?);
            }
            rows
        }
    };
    // Repos opened since: a small plot with the gift Garage.
    for o in offices.iter().rev() {
        if rows.iter().any(|r| r.workspace.as_deref() == Some(o.id.as_str())) {
            continue;
        }
        let i = free_plot(&mut rows, LotSizeName::Small);
        let (w, d) = dims(LotSizeName::Small);
        let lot = design::fit(design::first(false, vec![]), w, d).expect("the garage fits a small plot");
        (rows[i].owned, rows[i].workspace, rows[i].gift) = (true, Some(o.id.clone()), true);
        writes.push((lot_key(&rows[i].id), json(&lot)?));
    }
    writes.push((SLOTS.into(), json(&rows)?));
    store.kv_write(&writes)?;
    Ok(rows)
}

fn find(rows: &[Row], slot: &str) -> Result<usize> {
    rows.iter().position(|r| r.id == slot).ok_or_else(|| anyhow!("no such plot"))
}

fn home(rows: &[Row], ws: &str) -> Result<usize> {
    rows.iter().position(|r| r.workspace.as_deref() == Some(ws)).ok_or_else(|| anyhow!("this repo has no plot"))
}

/// A plot's lot: as stored, or the bare plot.
fn read_lot(store: &Store, row: &Row) -> Result<Lot> {
    match store.kv_get(&lot_key(&row.id))? {
        Some(v) => Ok(serde_json::from_str(&v)?),
        None => {
            let (w, d) = dims(row.size);
            Ok(design::empty(w, d))
        }
    }
}

/// The repo's lot (on its plot).
pub(super) fn lot_of(store: &Store, ws: &str) -> Result<Lot> {
    let rows = slots(store)?;
    read_lot(store, &rows[home(&rows, ws)?])
}

/// A plot's lot, for drawing it from outside: a bare plot if it isn't yours.
pub(super) fn lot_on(store: &Store, slot: &str) -> Result<Lot> {
    let rows = slots(store)?;
    let row = &rows[find(&rows, slot)?];
    if !row.owned {
        let (w, d) = dims(row.size);
        return Ok(design::empty(w, d));
    }
    read_lot(store, row)
}

/// Change the repo's lot with `f` and pay for what changed (`price::diff`): checked first, the
/// plot keeps its size and design name, gifts stay gifts (and nothing becomes one).
pub(super) fn change(store: &Store, ws: &str, what: &str, f: impl FnOnce(&mut Lot) -> Result<()>) -> Result<LotSaved> {
    let _m = wallet::money();
    let rows = slots(store)?;
    let row = &rows[home(&rows, ws)?];
    let old = read_lot(store, row)?;
    let mut new = old.clone();
    f(&mut new)?;
    let mut lot = design::validate(new)?;
    let (w, d) = dims(row.size);
    ensure!(lot.size.w == w && lot.size.d == d, "this plot is {w} × {d} m: a lot keeps its size");
    lot.template = old.template.clone();
    lot.gift = kept_gifts(&old, &lot);
    let cost = price::diff(&old, &lot);
    let wallet = wallet::pay(store, cost, what, vec![(lot_key(&row.id), json(&lot)?)])?;
    Ok(LotSaved { lot, wallet, cost })
}

/// Build mode's save: the whole lot, priced against the one stored.
pub(super) fn save_lot_of(store: &Store, ws: &str, lot: Lot) -> Result<LotSaved> {
    change(store, ws, "build", |l| {
        *l = lot;
        Ok(())
    })
}

/// The map: every plot, with the repo on it and what it's worth.
pub(super) fn list(store: &Store) -> Result<Vec<LotSlot>> {
    let rows = slots(store)?;
    let offices: HashMap<String, Office> = store.offices()?.into_iter().map(|o| (o.id.clone(), o)).collect();
    rows.iter()
        .map(|r| {
            let lot = if r.owned { Some(read_lot(store, r)?) } else { None };
            let office = r.workspace.as_ref().and_then(|w| offices.get(w));
            Ok(LotSlot {
                id: r.id.clone(),
                x: r.x,
                z: r.z,
                size: r.size,
                owned: r.owned,
                workspace_id: r.workspace.clone(),
                name: office.map(|o| o.name.clone()),
                crew: office.map_or(0, |o| o.crew),
                price: price::land(r.size),
                template: lot.as_ref().map(|l| l.template.clone()).filter(|t| !t.is_empty()),
                gift: r.gift,
                sells_for: lot.map_or(0, |l| if r.gift { 0 } else { price::land_back(r.size) } + price::sell_value(&l)),
            })
        })
        .collect()
}

/// Buy a plot, bare or with a ready-made design on it (`template`), at the plot's price plus
/// everything on it at build prices. On a plot already yours with nothing on it (a new repo's
/// bare plot), the design alone.
pub(super) fn buy(store: &Store, slot: &str, template: Option<&str>) -> Result<Wallet> {
    let _m = wallet::money();
    let mut rows = slots(store)?;
    let i = find(&rows, slot)?;
    let size = rows[i].size;
    let (w, d) = dims(size);
    let made = match template {
        None => None,
        Some(name) => {
            let t = design::template(name).ok_or_else(|| anyhow!("no such design: {name}"))?;
            let mut lot = design::fit(t, w, d).ok_or_else(|| anyhow!("the {name} doesn't fit on a {w} × {d} m plot"))?;
            lot.gift.clear();
            Some(lot)
        }
    };
    if !rows[i].owned {
        let lot = made.unwrap_or_else(|| design::empty(w, d));
        let cost = price::land(size) + price::value(&lot);
        (rows[i].owned, rows[i].gift) = (true, false);
        return wallet::pay(store, cost, &format!("plot {slot}"), vec![(SLOTS.into(), json(&rows)?), (lot_key(slot), json(&lot)?)]);
    }
    let Some(lot) = made else { bail!("that plot is already yours") };
    let old = read_lot(store, &rows[i])?;
    ensure!(design::is_empty(&old), "something already stands there: knock it down first");
    let cost = price::diff(&old, &lot);
    wallet::pay(store, cost, &format!("building on {slot}"), vec![(lot_key(slot), json(&lot)?)])
}

/// Sell a plot: 80% of the land (nothing for a free one) and everything on it at its kind's
/// rate, gifts for nothing. Refused while a repo's office is there.
pub(super) fn sell(store: &Store, slot: &str) -> Result<Wallet> {
    let _m = wallet::money();
    let mut rows = slots(store)?;
    let i = find(&rows, slot)?;
    ensure!(rows[i].owned, "that plot isn't yours");
    if let Some(ws) = &rows[i].workspace {
        let name = store.workspace(ws)?.map_or_else(|| "a repo".to_string(), |w| w.name);
        bail!("{name}'s office is there: move it out first");
    }
    let lot = read_lot(store, &rows[i])?;
    let back = if rows[i].gift { 0 } else { price::land_back(rows[i].size) } + price::sell_value(&lot);
    (rows[i].owned, rows[i].gift) = (false, false);
    wallet::pay(store, -back, &format!("sold plot {slot}"), vec![(SLOTS.into(), json(&rows)?), (lot_key(slot), None)])
}

/// Move a repo's office to another plot of yours with no office on it. Its furniture comes
/// along where it stands, if that's on the new plot (the rest, and what was stored, goes to
/// storage there); the building stays behind, on the old plot, with no office in it. Nothing is
/// bought or sold.
pub(super) fn move_office(store: &Store, ws: &str, to: &str) -> Result<Vec<LotSlot>> {
    let _m = wallet::money();
    let mut rows = slots(store)?;
    let (from, to) = (home(&rows, ws)?, find(&rows, to)?);
    ensure!(from != to, "the office is already there");
    ensure!(rows[to].owned, "buy that plot first");
    ensure!(rows[to].workspace.is_none(), "another office is there");
    let (mut old, mut new) = (read_lot(store, &rows[from])?, read_lot(store, &rows[to])?);
    let gift: HashSet<String> = old.gift.iter().cloned().collect();
    let mut taken = design::ids(&new);
    let (w, d) = (new.size.w, new.size.d);
    let standing = std::mem::take(&mut old.furniture).into_iter().map(|p| (p, true));
    let stored = std::mem::take(&mut old.storage).into_iter().map(|p| (p, false));
    for (mut p, up) in standing.chain(stored) {
        let was_gift = gift.contains(&p.uid);
        if taken.contains(&p.uid) {
            p.uid = new_id();
        }
        taken.insert(p.uid.clone());
        if was_gift {
            new.gift.push(p.uid.clone());
        }
        if up && inside(&p, w, d) { new.furniture.push(p) } else { new.storage.push(p) }
    }
    let left = design::ids(&old);
    old.gift.retain(|id| left.contains(id));
    let new = design::validate(new)?;
    rows[to].workspace = rows[from].workspace.take();
    store.kv_write(&[(SLOTS.into(), json(&rows)?), (lot_key(&rows[from].id), json(&old)?), (lot_key(&rows[to].id), json(&new)?)])?;
    list(store)
}

/// The developer panel's reset: the repo's lot back to the gift Garage; what stood there sells
/// back at the usual rates (gifts for nothing).
pub(super) fn reset(store: &Store, ws: &str) -> Result<Wallet> {
    let _m = wallet::money();
    let rows = slots(store)?;
    let row = &rows[home(&rows, ws)?];
    let old = read_lot(store, row)?;
    let (w, d) = dims(row.size);
    let lot = design::fit(design::first(false, vec![]), w, d).expect("the garage fits every plot");
    wallet::pay(store, -price::sell_value(&old), "reset the office", vec![(lot_key(&row.id), json(&lot)?)])
}

impl Inner {
    pub(super) fn lot(&self, ws: &str) -> Result<Lot> {
        lot_of(&self.store, ws)
    }

    pub(super) fn save_lot(&self, ws: &str, lot: Lot) -> Result<LotSaved> {
        save_lot_of(&self.store, ws, lot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(store: &Store, name: &str) -> String {
        let w = Workspace {
            id: new_id(),
            repo_path: format!("/tmp/{name}"),
            name: name.into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&w).unwrap();
        w.id
    }

    /// An agent let go long ago: the repo had a crew before lots.
    fn with_history(store: &Store, ws: &str) {
        let id = new_id();
        let agent = Agent {
            id: id.clone(),
            name: "Old".into(),
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
            archived: true,
            usage: Usage::default(),
            career: Career::default(),
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        let row = EventRow { id: 0, ts: now_ms(), agent_id: None, task_id: None, run_id: None, kind: EventKind::AgentChanged { agent } };
        store.append_event(Some(ws), &row).unwrap();
    }

    fn piece(uid: &str, item: &str, x: f64, z: f64) -> Placed {
        Placed { uid: uid.into(), item: item.into(), x, y: 0.0, z, rot: 0.0 }
    }

    fn coins(store: &Store) -> i64 {
        wallet::wallet(store).unwrap().balance
    }

    fn slot<'a>(all: &'a [LotSlot], ws: &str) -> &'a LotSlot {
        all.iter().find(|s| s.workspace_id.as_deref() == Some(ws)).unwrap()
    }

    #[test]
    fn what_came_before_lots_moves_in() {
        let store = Store::open_in_memory().unwrap();
        // An old repo with Build/Buy furniture and no lot yet; a newer one with its Garage
        // already made (and a sofa mirrored into it, bought into decor too).
        let (old, new) = (ws(&store, "old"), ws(&store, "new"));
        with_history(&store, &old);
        let sofa = piece("sofa", "kenney.loungeSofa", 19.5, 18.75);
        store.kv_set(&format!("decor:{old}"), &serde_json::to_string(&vec![sofa.clone()]).unwrap()).unwrap();
        let mut garage = design::first(false, vec![]);
        let chair = piece("chair", "kenney.chair", 2.0, 6.0);
        let far = piece("far", "kenney.chair", 2.0, 20.0);
        garage.furniture.push(chair.clone());
        store.kv_set(&format!("lot:{new}"), &serde_json::to_string(&garage).unwrap()).unwrap();
        store.kv_set(&format!("decor:{new}"), &serde_json::to_string(&vec![chair.clone(), far.clone()]).unwrap()).unwrap();
        store.kv_set("decor:gone", &serde_json::to_string(&vec![piece("x", "kenney.chair", 1.0, 1.0)]).unwrap()).unwrap();
        wallet::grant(&store, 2000, "test").unwrap();

        // The coins stay where they were: what decor cost is in the ledger now.
        let sofa_price = crate::career::price("kenney.loungeSofa").unwrap();
        let w = wallet::wallet(&store).unwrap();
        assert_eq!((w.spent, w.balance), (sofa_price + 3 * 25, 2000 - sofa_price - 75));
        assert!(store.kv_prefixed("decor:").unwrap().is_empty() && store.kv_get(&format!("lot:{new}")).unwrap().is_none());

        let all = list(&store).unwrap();
        let (a, b) = (slot(&all, &old), slot(&all, &new));
        assert_eq!((a.size, a.owned, a.gift, a.template.as_deref(), a.name.as_deref()), (LotSizeName::Large, true, true, Some("studio"), Some("old")));
        assert_eq!((b.size, b.template.as_deref()), (LotSizeName::Small, Some("garage")));
        // A free plot and a gift building: only the bought furniture sells for anything.
        assert_eq!((a.sells_for, b.sells_for), (sofa_price, 50));
        let lot = lot_of(&store, &old).unwrap();
        assert!(lot.furniture.contains(&sofa) && !lot.gift.contains(&sofa.uid) && lot.gift.contains(&"w1".to_string()));
        let lot = lot_of(&store, &new).unwrap();
        assert_eq!(lot.furniture.iter().filter(|p| p.uid == "chair").count(), 1);
        assert_eq!(lot.storage, vec![far], "off the garage: put away");
        assert_eq!(all.len(), 12);
        assert_eq!(all.iter().filter(|s| s.owned).count(), 2);
        // Done once: the second read changes nothing.
        assert_eq!(list(&store).unwrap(), all);
        assert_eq!(wallet::wallet(&store).unwrap(), w);
    }

    #[test]
    fn every_new_repo_gets_a_small_plot_with_a_gift_garage() {
        let store = Store::open_in_memory().unwrap();
        let a = ws(&store, "a");
        let lot = lot_of(&store, &a).unwrap();
        assert_eq!(lot.template, "garage");
        assert_eq!(lot.gift.len(), design::ids(&lot).len());
        let b = ws(&store, "b");
        let lot = lot_of(&store, &b).unwrap();
        assert_eq!(lot.template, "garage", "every repo can work: the second gets one too");
        let all = list(&store).unwrap();
        for w in [&a, &b] {
            let s = slot(&all, w);
            assert_eq!((s.size, s.owned, s.gift), (LotSizeName::Small, true, true));
        }
        assert_eq!(coins(&store), 0, "nothing was paid, and nothing sells: {all:?}");
        assert!(all.iter().all(|s| s.sells_for == 0));
        assert!(buy(&store, &slot(&all, &b).id, Some("garage")).is_err(), "something stands there");
        assert!(buy(&store, &slot(&all, &b).id, None).is_err(), "already yours");
    }

    #[test]
    fn saving_prices_the_change_and_keeps_gifts_gifts() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store, "repo");
        let mut lot = lot_of(&store, &w).unwrap();
        // Try to sneak the gift marks off, and to mark something new as a gift.
        lot.gift.clear();
        lot.gift.push("bought".into());
        lot.furniture.retain(|p| p.uid != "g6");
        lot.furniture.push(piece("bought", "kenney.chair", 2.0, 6.0));
        assert!(save_lot_of(&store, &w, lot.clone()).unwrap_err().to_string().contains("not enough coins"));
        assert_eq!(lot_of(&store, &w).unwrap().furniture.len(), 6, "refused: nothing changed");
        wallet::grant(&store, 100, "test").unwrap();
        let saved = save_lot_of(&store, &w, lot).unwrap();
        assert_eq!((saved.cost, saved.wallet.balance), (25, 75), "the chair; the gift mailbox sold for nothing");
        assert!(saved.lot.gift.contains(&"g1".to_string()) && saved.lot.gift.contains(&"w1".to_string()));
        assert!(!saved.lot.gift.contains(&"g6".to_string()) && !saved.lot.gift.contains(&"bought".to_string()));
        assert_eq!(lot_of(&store, &w).unwrap(), saved.lot);
        // A wall: 10 a metre; knocked down, half back. A gift wall knocked down: nothing.
        let mut lot = saved.lot.clone();
        lot.walls.push(LotWall { id: "w5".into(), x0: 5.0, z0: 0.0, x1: 5.0, z1: 4.0, kind: WallKind::Solid });
        assert_eq!(save_lot_of(&store, &w, lot.clone()).unwrap().cost, 40);
        lot.walls.retain(|x| x.id != "w5" && x.id != "w2");
        lot.windows.clear();
        let s = save_lot_of(&store, &w, lot).unwrap();
        assert_eq!((s.cost, s.wallet.balance), (-20, 55));
        // A bad lot, or one of another size, is refused and nothing changes.
        let mut bad = s.lot.clone();
        bad.size.w = 0.0;
        assert!(save_lot_of(&store, &w, bad).is_err());
        let mut big = s.lot.clone();
        big.size.w = 12.0;
        assert!(save_lot_of(&store, &w, big).unwrap_err().to_string().contains("keeps its size"));
        assert_eq!(lot_of(&store, &w).unwrap(), s.lot);
    }

    #[test]
    fn plots_are_bought_and_sold() {
        let store = Store::open_in_memory().unwrap();
        let home = ws(&store, "home");
        let all = list(&store).unwrap();
        let medium = all.iter().find(|s| !s.owned && s.size == LotSizeName::Medium).unwrap().clone();
        assert_eq!((medium.price, medium.sells_for, medium.template.clone()), (1200, 0, None));
        assert!(design::is_empty(&lot_on(&store, &medium.id).unwrap()));
        assert!(buy(&store, &medium.id, None).unwrap_err().to_string().contains("not enough coins"));
        assert!(buy(&store, &medium.id, Some("studio")).unwrap_err().to_string().contains("doesn't fit"));
        let price = price::ready_made(LotSizeName::Medium, Some("garage")).unwrap();
        wallet::grant(&store, price, "test").unwrap();
        assert_eq!(buy(&store, &medium.id, Some("garage")).unwrap().balance, 0);
        let s = list(&store).unwrap().into_iter().find(|s| s.id == medium.id).unwrap();
        assert_eq!((s.owned, s.gift, s.template.as_deref()), (true, false, Some("garage")));
        // Selling: 80% of the land, the building at half, the furniture in full; never more
        // than it cost.
        let g = lot_on(&store, &medium.id).unwrap();
        assert_eq!(s.sells_for, 960 + price::sell_value(&g));
        assert!(s.sells_for < price);
        assert!(sell(&store, &slot(&list(&store).unwrap(), &home).id).unwrap_err().to_string().contains("home's office is there"));
        assert_eq!(sell(&store, &medium.id).unwrap().balance, s.sells_for);
        assert!(sell(&store, &medium.id).is_err(), "not yours any more");
        assert!(!list(&store).unwrap().into_iter().find(|s| s.id == medium.id).unwrap().owned);
    }

    #[test]
    fn an_office_moves_with_its_furniture() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store, "repo");
        wallet::grant(&store, 10_000, "test").unwrap();
        let from = slot(&list(&store).unwrap(), &w).id.clone();
        super::change(&store, &w, "test", |lot| {
            lot.furniture.push(piece("sofa", "kenney.loungeSofa", 3.0, 6.0));
            lot.storage.push(piece("spare", "kenney.chair", 0.0, 0.0));
            Ok(())
        })
        .unwrap();
        let before = coins(&store);
        // To a plot that isn't yours, or onto itself: no.
        let medium = list(&store).unwrap().into_iter().find(|s| !s.owned && s.size == LotSizeName::Medium).unwrap().id;
        assert!(move_office(&store, &w, &medium).unwrap_err().to_string().contains("buy that plot"));
        assert!(move_office(&store, &w, &from).is_err());
        buy(&store, &medium, None).unwrap();
        let paid = coins(&store);
        let all = move_office(&store, &w, &medium).unwrap();
        assert_eq!(coins(&store), paid, "moving is free");
        assert_eq!(slot(&all, &w).id, medium);
        let old = all.iter().find(|s| s.id == from).unwrap();
        assert!(old.owned && old.workspace_id.is_none() && old.template.as_deref() == Some("garage"));
        // The building stays; the furniture came along, gifts still gifts.
        let left = lot_on(&store, &from).unwrap();
        assert!(left.furniture.is_empty() && left.storage.is_empty() && left.walls.len() == 4);
        assert!(left.gift.contains(&"w1".to_string()) && !left.gift.contains(&"g1".to_string()));
        let here = lot_of(&store, &w).unwrap();
        assert_eq!(here.furniture.len(), 7);
        assert_eq!(here.storage.len(), 1);
        assert!(here.gift.contains(&"g1".to_string()) && !here.gift.contains(&"sofa".to_string()));
        // The old plot sells now: its free land and gift walls for nothing.
        assert_eq!(sell(&store, &from).unwrap().balance, paid);
        assert!(paid < before);
        // Back to a small plot, with a building in the way: what isn't on it goes to storage.
        let small = list(&store).unwrap().into_iter().find(|s| !s.owned && s.size == LotSizeName::Small).unwrap().id;
        buy(&store, &small, Some("garage")).unwrap();
        super::change(&store, &w, "test", |lot| {
            lot.furniture.push(piece("far", "kenney.chair", 14.0, 11.0));
            Ok(())
        })
        .unwrap();
        move_office(&store, &w, &small).unwrap();
        let here = lot_of(&store, &w).unwrap();
        assert_eq!(here.furniture.len(), 6 + 7, "the garage's own and the office's");
        assert_eq!(here.storage.iter().map(|p| p.uid.as_str()).collect::<Vec<_>>().len(), 2);
        assert!(here.storage.iter().any(|p| p.item == "kenney.chair" && p.x == 14.0));
        design::validate(here).unwrap();
    }

    #[test]
    fn resetting_puts_the_gift_garage_back() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store, "repo");
        with_history(&store, &w);
        wallet::grant(&store, 1000, "test").unwrap();
        super::change(&store, &w, "test", |lot| {
            lot.furniture.push(piece("sofa", "kenney.loungeSofa", 3.0, 6.0));
            lot.walls.push(LotWall { id: "n1".into(), x0: 1.0, z0: 1.0, x1: 1.0, z1: 3.0, kind: WallKind::Solid });
            Ok(())
        })
        .unwrap();
        let sofa = crate::career::price("kenney.loungeSofa").unwrap();
        assert_eq!(coins(&store), 1000 - sofa - 20);
        let w2 = reset(&store, &w).unwrap();
        assert_eq!(w2.balance, 1000 - 10, "the sofa back in full, the wall at half, the studio's gifts nothing");
        let lot = lot_of(&store, &w).unwrap();
        assert_eq!((lot.template.as_str(), lot.size.w, lot.size.d), ("garage", 28.0, 24.0), "on its own large plot");
        assert_eq!(lot.gift.len(), design::ids(&lot).len());
        design::validate(lot).unwrap();
    }

    /// PROGRESSION.md §4: for any sequence of buy, sell, move and rebuild, coins after ≤ coins
    /// before unless real work earned some in between.
    #[test]
    fn no_sequence_of_buying_selling_moving_and_building_makes_coins() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = (ws(&store, "a"), ws(&store, "b"));
        with_history(&store, &a);
        wallet::grant(&store, 50_000, "test").unwrap();
        let start = coins(&store);
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        let templates = design::template_names();
        let mut done = [0; 6];
        for step in 0..300 {
            let all = list(&store).unwrap();
            let pick = |s: &[LotSlot], n: u64| s[(n as usize) % s.len()].id.clone();
            let ws = if next(2) == 0 { &a } else { &b };
            let what = next(6);
            let r = match what {
                0 => {
                    let t = if next(2) == 0 { None } else { Some(templates[next(templates.len() as u64) as usize]) };
                    buy(&store, &pick(&all, next(100)), t).map(|_| ())
                }
                1 => sell(&store, &pick(&all, next(100))).map(|_| ()),
                2 => move_office(&store, ws, &pick(&all, next(100))).map(|_| ()),
                3 => super::change(&store, ws, "test", |lot| {
                    let (w, d) = (lot.size.w, lot.size.d);
                    lot.furniture.push(piece(&format!("p{step}"), "office.desk", (next(w as u64) as f64).min(w), (next(d as u64) as f64).min(d)));
                    lot.walls.push(LotWall { id: format!("n{step}"), x0: 0.5, z0: 0.5, x1: 0.5, z1: 1.0 + next(3) as f64, kind: WallKind::Glass });
                    Ok(())
                })
                .map(|_| ()),
                4 => super::change(&store, ws, "test", |lot| {
                    if !lot.furniture.is_empty() {
                        let i = next(lot.furniture.len() as u64) as usize;
                        let p = lot.furniture.remove(i);
                        if next(2) == 0 {
                            lot.storage.push(p);
                        }
                    }
                    if let Some(i) = (!lot.walls.is_empty()).then(|| next(lot.walls.len() as u64) as usize) {
                        let id = lot.walls.remove(i).id;
                        lot.doors.retain(|d| d.wall != id);
                        lot.windows.retain(|d| d.wall != id);
                        lot.paint.retain(|d| d.wall != id);
                    }
                    Ok(())
                })
                .map(|_| ()),
                _ => reset(&store, ws).map(|_| ()),
            };
            // Nothing is earned here: coins, plus everything sold back, never pass what you had.
            let now = coins(&store);
            let worth: i64 = list(&store).unwrap().iter().map(|s| s.sells_for).sum();
            assert!(now + worth <= start, "step {step}: {now} + {worth} back, from {start}");
            done[what as usize] += r.is_ok() as usize;
        }
        assert!(done.iter().all(|&n| n > 5), "every kind of step happened: {done:?}");
    }
}
