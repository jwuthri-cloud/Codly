//! Build mode's lots: the ready-made designs (`src/office/lots/*.json`, shared with the office
//! frontend), a repo's first lot, putting a design on a plot, and the checks a saved lot must
//! pass. Prices are `crate::price`. Pure: the orchestrator
//! keeps each repo's lot in kv `lot:{ws}` (`orchestrator/lots.rs`), and `src/office/layout.ts`
//! turns it into rooms, stations and the walking grid.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail, ensure};

use crate::model::*;

// `TEMPLATES`: every ready-made design in `src/office/lots/` (build.rs): the Studio (today's
// 9-room floor plan, made once from the hand-built one), the Garage (a new repo's first office:
// one room, a roll-up door, the Lead's desk and two more), and any added since.
include!(concat!(env!("OUT_DIR"), "/templates.rs"));

/// Schema version.
pub const VERSION: u32 = 1;
/// Walls and floor tiles run on this grid (metres).
pub const TILE: f64 = 0.5;
/// The biggest lot side (metres).
pub const MAX_SIDE: f64 = 64.0;
/// Floor materials (`textures.ts` paints each).
pub const FLOORS: &[&str] = &["oak", "walnut", "walnut-planks", "carpet-sage", "carpet-blue", "tile-cool", "tile-warm", "terrazzo", "concrete"];

/// The floor of a bare plot, before anything is laid on it.
pub const BARE: &str = "concrete";

/// A ready-made design by name ("studio", "garage"…).
pub fn template(name: &str) -> Option<Lot> {
    let (_, raw) = TEMPLATES.iter().find(|(n, _)| *n == name)?;
    Some(serde_json::from_str(raw).expect("lot templates are valid"))
}

/// The names of every ready-made design.
pub fn template_names() -> Vec<&'static str> {
    TEMPLATES.iter().map(|(n, _)| *n).collect()
}

/// A bare plot of `w` × `d` metres: nothing built, a concrete floor.
pub fn empty(w: f64, d: f64) -> Lot {
    Lot {
        v: VERSION,
        template: String::new(),
        size: LotSize { w, d },
        walls: vec![],
        doors: vec![],
        windows: vec![],
        floors: LotFloors { tile: TILE, default: BARE.into(), rects: vec![] },
        paint: vec![],
        labels: vec![],
        furniture: vec![],
        gift: vec![],
        storage: vec![],
    }
}

/// Nothing stands on it (no walls, doors, windows or furniture, nothing stored).
pub fn is_empty(lot: &Lot) -> bool {
    lot.walls.is_empty() && lot.doors.is_empty() && lot.windows.is_empty() && lot.furniture.is_empty() && lot.storage.is_empty()
}

/// A design put down on a plot of `w` × `d`: centred across, against the south edge (the street,
/// where its front door is). Its own floor becomes a rect on the bare plot. None if it's too big.
pub fn fit(mut lot: Lot, w: f64, d: f64) -> Option<Lot> {
    let (lw, ld) = (lot.size.w, lot.size.d);
    if lw > w + 1e-6 || ld > d + 1e-6 {
        return None;
    }
    if (lw - w).abs() < 1e-6 && (ld - d).abs() < 1e-6 {
        return Some(lot);
    }
    let dx = (((w - lw) / 2.0) / TILE).floor() * TILE;
    let dz = d - ld;
    for wall in &mut lot.walls {
        (wall.x0, wall.x1, wall.z0, wall.z1) = (wall.x0 + dx, wall.x1 + dx, wall.z0 + dz, wall.z1 + dz);
    }
    for p in lot.furniture.iter_mut().chain(lot.storage.iter_mut()) {
        (p.x, p.z) = (p.x + dx, p.z + dz);
    }
    for r in &mut lot.floors.rects {
        (r.x, r.z) = (r.x + dx, r.z + dz);
    }
    for l in &mut lot.labels {
        (l.x, l.z) = (l.x + dx, l.z + dz);
    }
    if lot.floors.default != BARE {
        let m = std::mem::replace(&mut lot.floors.default, BARE.into());
        lot.floors.rects.insert(0, FloorRect { x: dx, z: dz, w: lw, d: ld, m });
    }
    lot.size = LotSize { w, d };
    Some(lot)
}

/// Everything in a lot that can be a gift: walls, doors, windows and furniture (stored too).
pub fn ids(lot: &Lot) -> HashSet<String> {
    lot.walls
        .iter()
        .map(|w| w.id.clone())
        .chain(lot.doors.iter().map(|d| d.id.clone()))
        .chain(lot.windows.iter().map(|w| w.id.clone()))
        .chain(lot.furniture.iter().chain(&lot.storage).map(|p| p.uid.clone()))
        .collect()
}

/// The gift marks a saved lot keeps: what came free and is still there (furniture still the
/// same piece). Nothing else becomes a gift, so a gift can never be sold for coins.
pub fn kept_gifts(old: &Lot, new: &Lot) -> Vec<String> {
    let item = |lot: &Lot, uid: &str| lot.furniture.iter().chain(&lot.storage).find(|p| p.uid == uid).map(|p| p.item.clone());
    let now = ids(new);
    old.gift.iter().filter(|id| now.contains(*id) && item(old, id) == item(new, id)).cloned().collect()
}

/// A repo's first lot.One that already has a crew or furniture (`existing`) gets the Studio it
/// has always had, all a gift, with the furniture bought for it (`decor`: not a gift); a new
/// repo gets the Garage, all a gift.
pub fn first(existing: bool, decor: Vec<Placed>) -> Lot {
    let mut lot = template(if existing { "studio" } else { "garage" }).expect("built in");
    let mut gift: Vec<String> = ids(&lot).into_iter().collect();
    gift.sort();
    lot.gift = gift;
    if existing {
        let taken = ids(&lot);
        lot.furniture.extend(decor.into_iter().filter(|p| !taken.contains(&p.uid)));
    }
    lot
}

/// Whether a catalog piece is one the office builds itself (furniture.json `builtin`: desks,
/// racks, the boards…), as opposed to a model you buy.
pub fn builtin(item: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        #[serde(default)]
        builtin: Option<String>,
    }
    static CATALOG: std::sync::OnceLock<Vec<Entry>> = std::sync::OnceLock::new();
    let all = CATALOG.get_or_init(|| serde_json::from_str(include_str!("../../../src/office/furniture.json")).expect("furniture.json is valid"));
    all.iter().any(|e| e.id == item && e.builtin.is_some())
}

fn on_grid(v: f64) -> bool {
    v.is_finite() && ((v / TILE).round() * TILE - v).abs() < 1e-6
}

/// Check a lot and tidy it: every wall runs from its west or north end (doors and windows
/// follow), everything sits on the grid and inside the lot, ids are unique, materials and
/// furniture exist. Gift marks are the caller's (`orchestrator/lots.rs` keeps the old ones).
pub fn validate(mut lot: Lot) -> Result<Lot> {
    ensure!(lot.v == VERSION, "unknown lot version {}", lot.v);
    let (w, d) = (lot.size.w, lot.size.d);
    ensure!(w > 0.0 && d > 0.0 && w <= MAX_SIDE && d <= MAX_SIDE && on_grid(w) && on_grid(d), "a lot is 0.5 to {MAX_SIDE} m a side, on the 0.5 m grid");
    let inside = |x: f64, z: f64| x.is_finite() && z.is_finite() && (-1e-6..=w + 1e-6).contains(&x) && (-1e-6..=d + 1e-6).contains(&z);

    let mut seen = HashSet::new();
    let mut fresh = |id: &str, what: &str| -> Result<()> {
        ensure!(!id.is_empty() && id.len() <= 64, "{what} needs an id");
        ensure!(seen.insert(id.to_string()), "two things are called {id}");
        Ok(())
    };

    // Walls: along x or z, on the grid, inside; turned to run east or south.
    let mut flipped = HashSet::new();
    let mut len = BTreeMap::new();
    for wall in &mut lot.walls {
        fresh(&wall.id, "a wall")?;
        ensure!([wall.x0, wall.z0, wall.x1, wall.z1].into_iter().all(on_grid), "wall {} is off the 0.5 m grid", wall.id);
        ensure!(inside(wall.x0, wall.z0) && inside(wall.x1, wall.z1), "wall {} is outside the lot", wall.id);
        let along_x = wall.z0 == wall.z1 && wall.x0 != wall.x1;
        let along_z = wall.x0 == wall.x1 && wall.z0 != wall.z1;
        ensure!(along_x || along_z, "wall {} must run straight along x or z", wall.id);
        if wall.x1 < wall.x0 || wall.z1 < wall.z0 {
            (wall.x0, wall.x1, wall.z0, wall.z1) = (wall.x1, wall.x0, wall.z1, wall.z0);
            flipped.insert(wall.id.clone());
        }
        len.insert(wall.id.clone(), (wall.x1 - wall.x0) + (wall.z1 - wall.z0));
    }
    let kind: BTreeMap<String, WallKind> = lot.walls.iter().map(|w| (w.id.clone(), w.kind)).collect();

    // Doors and windows: on a wall, within it, not over each other.
    let mut spans: BTreeMap<String, Vec<(f64, f64, String)>> = BTreeMap::new();
    let mut opening = |id: &str, wall: &str, at: &mut f64, width: f64, what: &str| -> Result<()> {
        let Some(&l) = len.get(wall) else { bail!("{what} {id} is on no wall") };
        ensure!(width.is_finite() && width > 0.0 && at.is_finite(), "{what} {id} needs a width and a place");
        if flipped.contains(wall) {
            *at = l - *at;
        }
        let (a, b) = (*at - width / 2.0, *at + width / 2.0);
        ensure!(a >= -1e-6 && b <= l + 1e-6, "{what} {id} doesn't fit in its wall");
        let list = spans.entry(wall.to_string()).or_default();
        if let Some((_, _, other)) = list.iter().find(|(x, y, _)| a < *y - 1e-6 && b > *x + 1e-6) {
            bail!("{what} {id} overlaps {other}");
        }
        list.push((a, b, id.to_string()));
        Ok(())
    };
    for door in &mut lot.doors {
        fresh(&door.id, "a door")?;
        opening(&door.id, &door.wall, &mut door.at, door.width, "door")?;
    }
    for win in &mut lot.windows {
        fresh(&win.id, "a window")?;
        opening(&win.id, &win.wall, &mut win.at, win.width, "window")?;
        ensure!(kind.get(&win.wall) == Some(&WallKind::Exterior), "window {} must be in an outside wall", win.id);
    }

    // Floors: whole tiles of a known material.
    ensure!(lot.floors.tile == TILE, "floor tiles are {TILE} m");
    ensure!(FLOORS.contains(&lot.floors.default.as_str()), "no such floor: {}", lot.floors.default);
    for r in &lot.floors.rects {
        ensure!(FLOORS.contains(&r.m.as_str()), "no such floor: {}", r.m);
        ensure!([r.x, r.z, r.w, r.d].into_iter().all(on_grid) && r.w > 0.0 && r.d > 0.0, "floor tiles are whole 0.5 m tiles");
        ensure!(inside(r.x, r.z) && inside(r.x + r.w, r.z + r.d), "a floor goes outside the lot");
    }

    // Paint (sides go by direction, +z or +x, so a turned wall keeps them), labels.
    for p in &lot.paint {
        ensure!(kind.contains_key(&p.wall), "paint on no wall ({})", p.wall);
        ensure!(p.side == 1 || p.side == -1, "a wall has two sides, 1 and -1");
        ensure!(!p.paint.is_empty() && p.paint.len() <= 64, "paint needs a colour or a wallpaper");
    }
    for l in &lot.labels {
        ensure!(inside(l.x, l.z), "a room label is outside the lot");
        if let Some(n) = &l.name {
            ensure!(n.chars().count() <= 40, "a room name is 40 characters at most");
        }
    }

    // Furniture: catalog pieces, inside (or put away).
    for p in &lot.furniture {
        fresh(&p.uid, "a piece of furniture")?;
        ensure!(crate::career::price(&p.item).is_some(), "no such piece: {}", p.item);
        ensure!(inside(p.x, p.z) && p.rot.is_finite() && p.y.is_finite() && (0.0..=3.0).contains(&p.y), "{} is outside the lot", p.item);
    }
    for p in &lot.storage {
        fresh(&p.uid, "a piece of furniture")?;
        ensure!(crate::career::price(&p.item).is_some(), "no such piece: {}", p.item);
    }
    Ok(lot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(uid: &str, item: &str) -> Placed {
        Placed { uid: uid.into(), item: item.into(), x: 19.5, y: 0.0, z: 18.75, rot: 0.0 }
    }

    #[test]
    fn templates_are_valid_lots() {
        assert!(template_names().contains(&"studio") && template_names().contains(&"garage"));
        assert!(template("nope").is_none());
        for name in template_names() {
            let lot = template(name).unwrap();
            assert_eq!(lot.template, name);
            assert_eq!(validate(lot.clone()).unwrap(), lot, "{name} is already tidy");
        }
        let g = template("garage").unwrap();
        assert_eq!((g.size.w, g.size.d), (10.0, 8.0));
        assert_eq!(g.doors[0].kind, DoorKind::Rollup);
        let items: Vec<&str> = g.furniture.iter().map(|p| p.item.as_str()).collect();
        for need in ["office.leadDesk", "office.serverMachine", "office.shelf", "office.mailbox"] {
            assert!(items.contains(&need), "the garage has a {need}");
        }
        assert_eq!(items.iter().filter(|i| **i == "office.desk").count(), 2);
        assert!(g.furniture.iter().all(|p| builtin(&p.item)));
        assert!(!builtin("kenney.chair") && builtin("office.desk"));
    }

    #[test]
    fn a_repo_with_history_gets_the_studio_with_its_furniture() {
        let lot = first(true, vec![placed("sofa-1", "kenney.loungeSofa"), placed("s1", "kenney.chair")]);
        assert_eq!(lot.template, "studio");
        // Everything that came with it is a gift; what was bought is not.
        let gift: HashSet<_> = lot.gift.iter().cloned().collect();
        assert!(lot.walls.iter().all(|w| gift.contains(&w.id)) && lot.doors.iter().all(|d| gift.contains(&d.id)));
        let sofa = lot.furniture.iter().find(|p| p.uid == "sofa-1").unwrap();
        assert_eq!(sofa.item, "kenney.loungeSofa");
        assert!(!gift.contains("sofa-1"));
        assert!(lot.furniture.iter().filter(|p| p.uid != "sofa-1").all(|p| gift.contains(&p.uid)));
        // A bought piece can't take a gift's uid.
        assert_eq!(lot.furniture.iter().filter(|p| p.uid == "s1").count(), 1);
        assert_ne!(lot.furniture.iter().find(|p| p.uid == "s1").unwrap().item, "kenney.chair");
        validate(lot).unwrap();
    }

    #[test]
    fn a_design_fits_on_a_bigger_plot() {
        let studio = template("studio").unwrap();
        assert_eq!(fit(studio.clone(), 28.0, 24.0).unwrap(), studio, "its own size: as it is");
        assert!(fit(studio, 16.0, 12.0).is_none(), "too big");
        let g = fit(template("garage").unwrap(), 16.0, 12.0).unwrap();
        assert_eq!((g.size.w, g.size.d), (16.0, 12.0));
        let w3 = g.walls.iter().find(|w| w.id == "w3").unwrap();
        assert_eq!((w3.x0, w3.z0, w3.x1, w3.z1), (3.0, 12.0, 13.0, 12.0), "centred, its front on the street");
        assert_eq!(g.furniture[0].x, 8.0);
        validate(g).unwrap();
        // A design with its own floor keeps it where it stands.
        let mut oak = template("garage").unwrap();
        oak.floors.default = "oak".into();
        let f = fit(oak, 16.0, 12.0).unwrap();
        assert_eq!(f.floors.default, BARE);
        assert_eq!(f.floors.rects[0], FloorRect { x: 3.0, z: 4.0, w: 10.0, d: 8.0, m: "oak".into() });
        assert!(is_empty(&empty(10.0, 8.0)) && !is_empty(&f));
        validate(empty(10.0, 8.0)).unwrap();
    }

    #[test]
    fn a_new_repo_gets_the_garage_all_a_gift() {
        let lot = first(false, vec![]);
        assert_eq!(lot.template, "garage");
        assert_eq!(lot.gift.len(), ids(&lot).len());
    }

    #[test]
    fn a_saved_lot_is_checked_and_tidied() {
        let base = template("garage").unwrap();
        // A wall drawn east to west is turned round, its door with it.
        let mut lot = base.clone();
        let w = lot.walls.iter_mut().find(|w| w.id == "w3").unwrap();
        (w.x0, w.x1) = (w.x1, w.x0);
        lot.doors[0].at = 10.0 - lot.doors[0].at;
        assert_eq!(validate(lot).unwrap(), base);

        let bad = |f: &dyn Fn(&mut Lot), why: &str| {
            let mut lot = base.clone();
            f(&mut lot);
            let err = validate(lot).unwrap_err().to_string();
            assert!(err.contains(why), "{err} (wanted: {why})");
        };
        bad(&|l| l.v = 2, "version");
        bad(&|l| l.size.w = 10.3, "grid");
        bad(&|l| l.walls[0].z1 = 1.0, "straight");
        bad(&|l| l.walls[0].x1 = 12.0, "outside");
        bad(&|l| l.walls[1].id = "w1".into(), "two things");
        bad(&|l| l.doors[0].wall = "nope".into(), "no wall");
        bad(&|l| l.doors[0].at = 9.5, "doesn't fit");
        bad(
            &|l| {
                let mut d = l.doors[0].clone();
                d.id = "d2".into();
                d.at += 1.0;
                l.doors.push(d);
            },
            "overlaps",
        );
        bad(
            &|l| {
                l.walls.push(LotWall { id: "w5".into(), x0: 5.0, z0: 0.0, x1: 5.0, z1: 4.0, kind: WallKind::Solid });
                l.windows.push(LotWindow { id: "win2".into(), wall: "w5".into(), at: 2.0, width: 1.0, kind: WindowKind::Standard });
            },
            "outside wall",
        );
        bad(&|l| l.floors.default = "lava".into(), "no such floor");
        bad(&|l| l.floors.rects.push(FloorRect { x: 0.25, z: 0.0, w: 1.0, d: 1.0, m: "oak".into() }), "whole");
        bad(&|l| l.paint.push(WallPaint { wall: "w1".into(), side: 0, paint: "#ffffff".into() }), "two sides");
        bad(&|l| l.furniture[0].item = "office.nope".into(), "no such piece");
        bad(&|l| l.furniture[0].x = 11.0, "outside the lot");
    }
}
