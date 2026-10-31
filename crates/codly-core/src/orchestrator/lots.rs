//! Build mode: each repo's lot (`lot:{ws}`), made on the first read and checked on every save
//! (rules in `crate::lot`). Furniture bought through Build/Buy (`decor:{ws}`, careers.rs) is
//! mirrored into the lot as it changes.

use anyhow::Result;

use super::Inner;
use crate::model::*;
use crate::store::Store;

fn key(ws: &str) -> String {
    format!("lot:{ws}")
}

fn decor(store: &Store, ws: &str) -> Result<Vec<Placed>> {
    Ok(store.kv_get(&format!("decor:{ws}"))?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

/// The repo's lot; the first read makes it: the Studio for a repo with a crew, work or
/// furniture already (its decor kept), the Garage for a new one.
pub(super) fn lot_of(store: &Store, ws: &str) -> Result<Lot> {
    if let Some(v) = store.kv_get(&key(ws))? {
        return Ok(serde_json::from_str(&v)?);
    }
    let decor = decor(store, ws)?;
    let existing = !decor.is_empty() || !store.agents(ws)?.is_empty() || !store.archived_agents(ws)?.is_empty() || !store.tasks(ws)?.is_empty();
    let lot = crate::lot::first(existing, decor);
    store.kv_set(&key(ws), &serde_json::to_string(&lot)?)?;
    Ok(lot)
}

/// Check and store a lot. What came free stays a gift (and nothing else becomes one), so a
/// gift can never be sold for coins.
pub(super) fn save_lot_of(store: &Store, ws: &str, lot: Lot) -> Result<Lot> {
    let old = lot_of(store, ws)?;
    let mut lot = crate::lot::validate(lot)?;
    let now = crate::lot::ids(&lot);
    lot.gift = old.gift.into_iter().filter(|id| now.contains(id)).collect();
    // HOOK (build mode pricing, next step): diff `old` against `lot` here and charge or refund
    // the wallet for what changed (walls per metre, doors, windows, paint, furniture), at the
    // core's one price table; gifts (`old.gift`) refund 0. Until then a save is free, and the
    // furniture Build/Buy sells stays in `decor:{ws}` (careers.rs) as well, mirrored below.
    store.kv_set(&key(ws), &serde_json::to_string(&lot)?)?;
    Ok(lot)
}

/// Build/Buy changed the furniture bought (`decor:{ws}`): the lot's bought pieces follow.
/// The office's own pieces and gifts stay as they are.
pub(super) fn mirror_decor(store: &Store, ws: &str, placed: &[Placed]) -> Result<()> {
    let Some(v) = store.kv_get(&key(ws))? else { return Ok(()) };
    let mut lot: Lot = serde_json::from_str(&v)?;
    let gift: std::collections::HashSet<&String> = lot.gift.iter().collect();
    let mut furniture: Vec<Placed> = lot.furniture.iter().filter(|p| gift.contains(&p.uid) || crate::lot::builtin(&p.item)).cloned().collect();
    let taken: std::collections::HashSet<String> = furniture.iter().map(|p| p.uid.clone()).collect();
    furniture.extend(placed.iter().filter(|p| !taken.contains(&p.uid)).cloned());
    lot.furniture = furniture;
    store.kv_set(&key(ws), &serde_json::to_string(&lot)?)
}

impl Inner {
    pub(super) fn lot(&self, ws: &str) -> Result<Lot> {
        lot_of(&self.store, ws)
    }

    pub(super) fn save_lot(&self, ws: &str, lot: Lot) -> Result<Lot> {
        save_lot_of(&self.store, ws, lot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(store: &Store) -> String {
        let w = Workspace {
            id: new_id(),
            repo_path: "/tmp/repo".into(),
            name: "repo".into(),
            default_branch: "main".into(),
            current_branch: None,
            head: None,
            dirty: false,
            created_at: now_ms(),
        };
        store.upsert_workspace(&w).unwrap();
        w.id
    }

    #[test]
    fn a_new_repo_gets_a_garage_once() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let lot = lot_of(&store, &w).unwrap();
        assert_eq!(lot.template, "garage");
        assert_eq!(lot.gift.len(), crate::lot::ids(&lot).len());
        // Stored: the second read is the same lot, even once the repo has history.
        let sofa = Placed { uid: "sofa".into(), item: "kenney.loungeSofa".into(), x: 5.0, y: 0.0, z: 6.0, rot: 0.0 };
        store.kv_set(&format!("decor:{w}"), &serde_json::to_string(&vec![sofa]).unwrap()).unwrap();
        assert_eq!(lot_of(&store, &w).unwrap(), lot);
    }

    #[test]
    fn a_repo_with_furniture_gets_the_studio_with_it() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let sofa = Placed { uid: "sofa".into(), item: "kenney.loungeSofa".into(), x: 19.5, y: 0.0, z: 18.75, rot: 0.0 };
        store.kv_set(&format!("decor:{w}"), &serde_json::to_string(&vec![sofa.clone()]).unwrap()).unwrap();
        let lot = lot_of(&store, &w).unwrap();
        assert_eq!(lot.template, "studio");
        assert!(lot.furniture.contains(&sofa) && !lot.gift.contains(&sofa.uid));
        assert!(lot.gift.contains(&"w1".to_string()) && lot.gift.contains(&"s1".to_string()));

        // Build/Buy moves the sofa and buys a chair: the lot follows; its own pieces stay.
        let moved = Placed { x: 20.0, ..sofa.clone() };
        let chair = Placed { uid: "chair".into(), item: "kenney.chair".into(), x: 18.0, y: 0.0, z: 18.0, rot: 0.0 };
        mirror_decor(&store, &w, &[moved.clone(), chair.clone()]).unwrap();
        let lot2 = lot_of(&store, &w).unwrap();
        assert_eq!(lot2.furniture.len(), lot.furniture.len() + 1);
        assert!(lot2.furniture.contains(&moved) && lot2.furniture.contains(&chair) && !lot2.furniture.contains(&sofa));
    }

    #[test]
    fn saving_keeps_gifts_gifts() {
        let store = Store::open_in_memory().unwrap();
        let w = ws(&store);
        let mut lot = lot_of(&store, &w).unwrap();
        // Try to sneak the gift marks off, and to mark something new as a gift.
        lot.gift.clear();
        lot.gift.push("bought".into());
        lot.furniture.retain(|p| p.uid != "g6");
        lot.furniture.push(Placed { uid: "bought".into(), item: "kenney.chair".into(), x: 2.0, y: 0.0, z: 6.0, rot: 0.0 });
        let saved = save_lot_of(&store, &w, lot).unwrap();
        assert!(saved.gift.contains(&"g1".to_string()) && saved.gift.contains(&"w1".to_string()));
        assert!(!saved.gift.contains(&"g6".to_string()) && !saved.gift.contains(&"bought".to_string()));
        assert_eq!(lot_of(&store, &w).unwrap(), saved);
        // A bad lot is refused and nothing changes.
        let mut bad = saved.clone();
        bad.size.w = 0.0;
        assert!(save_lot_of(&store, &w, bad).is_err());
        assert_eq!(lot_of(&store, &w).unwrap(), saved);
    }
}
