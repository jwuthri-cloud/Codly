//! Build mode's prices (PROGRESSION.md §4, "Prices can't be gamed"): one table
//! (`src/office/lots/prices.json`, read by `src/office/price.ts` too) for plots, walls per metre,
//! doors, windows, floors and paint per m², with furniture at its catalog price
//! (`career::price`). Pure: what a lot is worth, what a change to it costs, what selling it gives
//! back. A lot is a list of priced parts, each with a key that stays the same while the part
//! does (a wall keeps its id when it moves; a floor rect is its size and material); comparing
//! two lots part by part is the whole of diff pricing.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::model::*;

pub const PRICES: &str = include_str!("../../../src/office/lots/prices.json");

#[derive(Debug, Deserialize)]
pub struct Table {
    pub lots: BTreeMap<LotSizeName, Plot>,
    pub wall_per_m: BTreeMap<String, f64>,
    /// Wall heights (metres) by kind, for the painted area.
    pub wall_h: BTreeMap<String, f64>,
    pub door: BTreeMap<String, i64>,
    pub window: BTreeMap<String, i64>,
    pub floor_per_m2: f64,
    pub paint_per_m2: f64,
    /// A bare plot's floor: laying it costs nothing.
    pub bare_floor: String,
    pub sell: Sell,
}

#[derive(Debug, Deserialize)]
pub struct Plot {
    pub w: f64,
    pub d: f64,
    pub price: i64,
}

/// What each kind sells back for, as a share of its price.
#[derive(Debug, Deserialize)]
pub struct Sell {
    pub furniture: f64,
    pub build: f64,
    pub paint: f64,
    pub lot: f64,
}

pub fn table() -> &'static Table {
    static T: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    T.get_or_init(|| serde_json::from_str(PRICES).expect("prices.json is valid"))
}

/// What a part sells back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Furniture,
    /// Walls, doors and windows.
    Build,
    /// Wall paint and floors.
    Paint,
}

/// One priced thing in a lot.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub key: String,
    /// Its id or uid, for the gift marks (paint and floors have none).
    pub id: Option<String>,
    pub kind: Kind,
    pub price: i64,
}

fn name<T: serde::Serialize>(t: &T) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn coins(v: f64) -> i64 {
    v.max(0.0).round() as i64
}

/// A plot's metres and price.
pub fn plot(size: LotSizeName) -> &'static Plot {
    &table().lots[&size]
}

/// The empty plot's price.
pub fn land(size: LotSizeName) -> i64 {
    plot(size).price
}

/// The smallest plot a `w` × `d` lot fits on (None: bigger than any).
pub fn size_for(w: f64, d: f64) -> Option<LotSizeName> {
    table().lots.iter().find(|(_, p)| w <= p.w + 1e-6 && d <= p.d + 1e-6).map(|(s, _)| *s)
}

/// What selling a part of this kind and price gives back (rounded down).
pub fn refund(kind: Kind, price: i64) -> i64 {
    let s = &table().sell;
    let rate = match kind {
        Kind::Furniture => s.furniture,
        Kind::Build => s.build,
        Kind::Paint => s.paint,
    };
    (price as f64 * rate).floor() as i64
}

/// Every priced part of a lot. Gift marks don't change prices (only refunds).
pub fn parts(lot: &Lot) -> Vec<Part> {
    let t = table();
    let mut out = vec![];
    let mut len = HashMap::new();
    for w in &lot.walls {
        let l = (w.x1 - w.x0).abs() + (w.z1 - w.z0).abs();
        let kind = name(&w.kind);
        len.insert(w.id.as_str(), (l, kind.clone()));
        out.push(Part { key: format!("wall:{}", w.id), id: Some(w.id.clone()), kind: Kind::Build, price: coins(l * t.wall_per_m.get(&kind).copied().unwrap_or(0.0)) });
    }
    for d in &lot.doors {
        out.push(Part { key: format!("door:{}", d.id), id: Some(d.id.clone()), kind: Kind::Build, price: t.door.get(&name(&d.kind)).copied().unwrap_or(0) });
    }
    for w in &lot.windows {
        out.push(Part { key: format!("window:{}", w.id), id: Some(w.id.clone()), kind: Kind::Build, price: t.window.get(&name(&w.kind)).copied().unwrap_or(0) });
    }
    for p in lot.furniture.iter().chain(&lot.storage) {
        out.push(Part { key: format!("piece:{}:{}", p.uid, p.item), id: Some(p.uid.clone()), kind: Kind::Furniture, price: crate::career::price(&p.item).unwrap_or(0) });
    }
    // Paint and floors have no ids: the same one twice is two parts.
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut nth = |key: String| {
        let n = seen.entry(key.clone()).or_default();
        *n += 1;
        format!("{key}#{n}")
    };
    for p in &lot.paint {
        let (l, kind) = len.get(p.wall.as_str()).cloned().unwrap_or_default();
        let area = l * t.wall_h.get(&kind).copied().unwrap_or(0.0);
        out.push(Part { key: nth(format!("paint:{}:{}:{}", p.wall, p.side, p.paint)), id: None, kind: Kind::Paint, price: coins(area * t.paint_per_m2) });
    }
    if lot.floors.default != t.bare_floor {
        let price = coins(lot.size.w * lot.size.d * t.floor_per_m2);
        out.push(Part { key: format!("floor:all:{}", lot.floors.default), id: None, kind: Kind::Paint, price });
    }
    for r in &lot.floors.rects {
        out.push(Part { key: nth(format!("floor:{}x{}:{}", r.w, r.d, r.m)), id: None, kind: Kind::Paint, price: coins(r.w * r.d * t.floor_per_m2) });
    }
    out
}

/// What everything on a lot costs to build (the plot aside), gifts at their price too.
pub fn value(lot: &Lot) -> i64 {
    parts(lot).iter().map(|p| p.price).sum()
}

/// What selling everything on a lot gives back, at each kind's rate; gifts give nothing.
pub fn sell_value(lot: &Lot) -> i64 {
    let gift: HashSet<&String> = lot.gift.iter().collect();
    parts(lot).iter().filter(|p| !p.id.as_ref().is_some_and(|id| gift.contains(id))).map(|p| refund(p.kind, p.price)).sum()
}

/// What selling a plot (not a gift) gives back for the land.
pub fn land_back(size: LotSizeName) -> i64 {
    (land(size) as f64 * table().sell.lot).floor() as i64
}

/// A plot with a ready-made design on it (None: empty): the plot plus everything on it, at the
/// prices of building it yourself. None if the design doesn't exist or doesn't fit.
pub fn ready_made(size: LotSizeName, template: Option<&str>) -> Option<i64> {
    let p = plot(size);
    let contents = match template {
        None => 0,
        Some(name) => value(&crate::lot::fit(crate::lot::template(name)?, p.w, p.d)?),
    };
    Some(p.price + contents)
}

/// What changing a lot from `old` to `new` costs (negative: it gives back). Something new costs
/// its price; something gone sells back at its kind's rate; something kept (same key: moved,
/// turned, put away) costs the difference if it grew and gives back the difference at its
/// kind's rate if it shrank. Gifts (`old.gift`) give back nothing.
pub fn diff(old: &Lot, new: &Lot) -> i64 {
    let gift: HashSet<&String> = old.gift.iter().collect();
    let gifted = |p: &Part| p.id.as_ref().is_some_and(|id| gift.contains(id));
    let before: HashMap<String, Part> = parts(old).into_iter().map(|p| (p.key.clone(), p)).collect();
    let after: HashMap<String, Part> = parts(new).into_iter().map(|p| (p.key.clone(), p)).collect();
    let mut cost = 0;
    for (k, n) in &after {
        match before.get(k) {
            None => cost += n.price,
            Some(o) if n.price > o.price => cost += n.price - o.price,
            Some(o) if n.price < o.price && !gifted(o) => cost -= refund(o.kind, o.price - n.price),
            Some(_) => {}
        }
    }
    for (k, o) in &before {
        if !after.contains_key(k) && !gifted(o) {
            cost -= refund(o.kind, o.price);
        }
    }
    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lot::{fit, template, template_names};

    fn chair(uid: &str) -> Placed {
        Placed { uid: uid.into(), item: "kenney.chair".into(), x: 2.0, y: 0.0, z: 2.0, rot: 0.0 }
    }

    /// A bought lot: nothing on it is a gift.
    fn bought(name: &str) -> Lot {
        Lot { gift: vec![], ..template(name).unwrap() }
    }

    #[test]
    fn the_table_prices_each_kind() {
        let g = template("garage").unwrap();
        // 36 m of exterior wall, a roll-up door, a window, six stations; a concrete floor.
        let furniture: i64 = g.furniture.iter().map(|p| crate::career::price(&p.item).unwrap()).sum();
        assert_eq!(value(&g), 36 * 10 + 100 + 30 + furniture);
        assert_eq!(sell_value(&g), 0, "a gift sells for nothing");
        assert_eq!(sell_value(&bought("garage")), (36 * 10 + 100 + 30) / 2 + furniture);
        assert_eq!((land(LotSizeName::Small), land_back(LotSizeName::Small)), (300, 240));
        assert_eq!(size_for(10.0, 8.0), Some(LotSizeName::Small));
        assert_eq!(size_for(11.0, 8.0), Some(LotSizeName::Medium));
        assert_eq!(size_for(28.0, 24.0), Some(LotSizeName::Large));
        assert_eq!(size_for(30.0, 8.0), None);
        // Paint: one side of a 10 m outside wall, 2.3 m high; a floor rect per m².
        let mut p = g.clone();
        p.paint.push(WallPaint { wall: "w1".into(), side: 1, paint: "#ffffff".into() });
        p.floors.rects.push(FloorRect { x: 0.0, z: 0.0, w: 2.0, d: 3.0, m: "oak".into() });
        assert_eq!(value(&p) - value(&g), 23 + 6);
    }

    #[test]
    fn a_ready_made_lot_costs_the_plot_plus_what_is_on_it() {
        for name in template_names() {
            let t = template(name).unwrap();
            let fits = size_for(t.size.w, t.size.d).unwrap();
            for size in [LotSizeName::Small, LotSizeName::Medium, LotSizeName::Large] {
                let p = plot(size);
                match fit(t.clone(), p.w, p.d) {
                    Some(lot) => assert_eq!(ready_made(size, Some(name)), Some(land(size) + value(&lot)), "{name} on {size:?}"),
                    None => assert!(size < fits && ready_made(size, Some(name)).is_none(), "{name} on {size:?}"),
                }
            }
        }
        assert_eq!(ready_made(LotSizeName::Medium, None), Some(1200));
        assert_eq!(ready_made(LotSizeName::Large, Some("nope")), None);
        // Bigger plot, same building: only the plot costs more.
        let (s, m) = (ready_made(LotSizeName::Small, Some("garage")).unwrap(), ready_made(LotSizeName::Medium, Some("garage")).unwrap());
        assert_eq!(m - s, land(LotSizeName::Medium) - land(LotSizeName::Small));
    }

    #[test]
    fn diff_prices_what_changed() {
        let base = bought("garage");
        assert_eq!(diff(&base, &base), 0);
        // A chair: its price; moved or put away: free; sold: all of it back.
        let mut a = base.clone();
        a.furniture.push(chair("c1"));
        assert_eq!(diff(&base, &a), 25);
        let mut b = a.clone();
        b.furniture.last_mut().unwrap().x = 5.0;
        assert_eq!(diff(&a, &b), 0);
        let mut c = a.clone();
        c.storage.push(c.furniture.pop().unwrap());
        assert_eq!(diff(&a, &c), 0);
        assert_eq!(diff(&a, &base), -25);
        // The same uid as something else: the old piece sold, the new one bought.
        let mut d = a.clone();
        d.furniture.last_mut().unwrap().item = "kenney.loungeSofa".into();
        assert_eq!(diff(&a, &d), crate::career::price("kenney.loungeSofa").unwrap() - 25);
        // A wall that grows costs the difference; shrinking gives half of it back.
        let mut e = base.clone();
        e.walls.push(LotWall { id: "w5".into(), x0: 5.0, z0: 0.0, x1: 5.0, z1: 4.0, kind: WallKind::Solid });
        assert_eq!(diff(&base, &e), 40);
        let mut f = e.clone();
        f.walls.last_mut().unwrap().z1 = 6.0;
        assert_eq!((diff(&e, &f), diff(&f, &e)), (20, -10));
        // Moving it is free; knocking it down gives half back; glass costs more.
        let mut g = e.clone();
        (g.walls.last_mut().unwrap().x0, g.walls.last_mut().unwrap().x1) = (6.0, 6.0);
        assert_eq!(diff(&e, &g), 0);
        assert_eq!(diff(&e, &base), -20);
        let mut h = e.clone();
        h.walls.last_mut().unwrap().kind = WallKind::Glass;
        assert_eq!(diff(&e, &h), 20);
        // A door, a floor, paint: doors give half back, paint and floors nothing.
        let mut i = base.clone();
        i.doors.push(LotDoor { id: "d2".into(), wall: "w1".into(), at: 2.0, width: 1.0, kind: DoorKind::Glass });
        i.floors.rects.push(FloorRect { x: 0.0, z: 0.0, w: 4.0, d: 4.0, m: "oak".into() });
        i.paint.push(WallPaint { wall: "w2".into(), side: 1, paint: "#ffcc00".into() });
        assert_eq!(diff(&base, &i), 120 + 16 + coins(8.0 * 2.3));
        assert_eq!(diff(&i, &base), -60);
        // A floor moved is the same floor; a repaint is new paint.
        let mut j = i.clone();
        j.floors.rects[0].x = 4.0;
        j.paint[0].paint = "#00ccff".into();
        assert_eq!(diff(&i, &j), coins(8.0 * 2.3));
    }

    #[test]
    fn gifts_give_back_nothing() {
        let gift = template("garage").unwrap();
        let mut empty = gift.clone();
        (empty.walls, empty.doors, empty.windows, empty.furniture) = (vec![], vec![], vec![], vec![]);
        assert_eq!(diff(&gift, &empty), 0, "knocked down and sold: nothing back");
        // Shrinking a gift wall gives nothing back either; growing it costs.
        let mut short = gift.clone();
        short.walls[0].x1 = 8.0;
        assert_eq!(diff(&gift, &short), 0);
        let mut put_away = gift.clone();
        put_away.storage.push(put_away.furniture.remove(0));
        assert_eq!(diff(&gift, &put_away), 0);
    }

    /// `src/office/price.ts` prices lots the same way, for build mode's live prices: both check
    /// themselves against this fixture (the TS side in `src/office/__tests__/price.test.ts`).
    /// `CODLY_WRITE_FIXTURES=1 cargo test -p codly-core price` rewrites it after a price change.
    #[test]
    fn the_office_prices_lots_the_same() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/office/__tests__/price-fixture.json");
        let mut templates = serde_json::Map::new();
        for name in template_names() {
            let t = template(name).unwrap();
            let ready: serde_json::Map<String, serde_json::Value> = [LotSizeName::Small, LotSizeName::Medium, LotSizeName::Large].into_iter().map(|s| (name_of(s), serde_json::json!(ready_made(s, Some(name))))).collect();
            templates.insert(name.into(), serde_json::json!({ "value": value(&t), "sell_gift": sell_value(&t), "sell_bought": sell_value(&bought(name)), "ready_made": ready }));
        }
        // Edits of a bought garage, and of the gift one.
        let base = bought("garage");
        let mut grown = base.clone();
        grown.walls[0].kind = WallKind::Glass;
        grown.walls.push(LotWall { id: "w5".into(), x0: 4.5, z0: 0.0, x1: 4.5, z1: 3.5, kind: WallKind::Half });
        grown.doors.push(LotDoor { id: "d2".into(), wall: "w5".into(), at: 1.5, width: 1.0, kind: DoorKind::Double });
        grown.paint.push(WallPaint { wall: "w5".into(), side: -1, paint: "#ffcc00".into() });
        grown.paint.push(WallPaint { wall: "w1".into(), side: 1, paint: "#ffcc00".into() });
        grown.floors.rects.push(FloorRect { x: 0.0, z: 0.0, w: 4.5, d: 3.5, m: "carpet-blue".into() });
        grown.floors.default = "oak".into();
        grown.furniture.push(chair("c1"));
        grown.storage.push(grown.furniture.remove(1));
        let mut shrunk = grown.clone();
        shrunk.walls[4].z1 = 2.0;
        shrunk.doors.pop();
        shrunk.walls.remove(1);
        shrunk.windows.clear();
        shrunk.furniture.retain(|p| p.uid != "g1");
        shrunk.floors.rects[0].x = 4.5;
        let gift = template("garage").unwrap();
        let mut gift_edit = shrunk.clone();
        gift_edit.gift = gift.gift.clone();
        let cases = [("build", &base, &grown), ("rebuild", &grown, &shrunk), ("undo", &shrunk, &base), ("gift", &gift, &Lot { gift: gift.gift.clone(), ..shrunk.clone() }), ("gift back", &gift_edit, &gift)];
        let diffs: Vec<serde_json::Value> = cases.iter().map(|(n, a, b)| serde_json::json!({ "name": n, "old": a, "new": b, "cost": diff(a, b) })).collect();
        let want = serde_json::json!({ "templates": templates, "diffs": diffs });
        if std::env::var_os("CODLY_WRITE_FIXTURES").is_some() {
            std::fs::write(path, serde_json::to_string_pretty(&want).unwrap() + "\n").unwrap();
        }
        let have: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or_default();
        assert!(have == want, "{path} is out of date: rerun with CODLY_WRITE_FIXTURES=1 (and check price.ts agrees)");
    }

    fn name_of(s: LotSizeName) -> String {
        name(&s)
    }

    /// The rule that matters: whatever you do to a lot, selling it all back never gives more
    /// than you paid. A long walk of random edits, priced step by step.
    #[test]
    fn no_sequence_of_edits_makes_coins() {
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        let items = ["kenney.chair", "kenney.loungeSofa", "office.desk", "office.plant"];
        let kinds = [WallKind::Solid, WallKind::Glass, WallKind::Half, WallKind::Exterior];
        for start in ["garage", "studio"] {
            for gifted in [true, false] {
                let mut lot = template(start).unwrap();
                if !gifted {
                    lot.gift.clear();
                }
                let (w, d) = (lot.size.w, lot.size.d);
                // Paid so far, counted as a ready-made purchase would charge it.
                let mut paid = if gifted { 0 } else { value(&lot) };
                for step in 0..400 {
                    let mut new = lot.clone();
                    let at = |n: u64| (n as f64) * 0.5;
                    match next(9) {
                        0 => new.furniture.push(Placed { uid: format!("p{step}"), item: items[next(4) as usize].into(), x: at(next(2 * w as u64)), y: 0.0, z: at(next(2 * d as u64)), rot: 0.0 }),
                        1 if !new.furniture.is_empty() => {
                            let i = next(new.furniture.len() as u64) as usize;
                            new.furniture.remove(i);
                        }
                        2 if !new.furniture.is_empty() => {
                            let i = next(new.furniture.len() as u64) as usize;
                            let p = new.furniture.remove(i);
                            new.storage.push(p);
                        }
                        3 => {
                            let (x, z, l) = (at(next(2 * w as u64)), at(next(2 * d as u64)), at(1 + next(8)));
                            new.walls.push(LotWall { id: format!("n{step}"), x0: x, z0: z, x1: x, z1: z + l, kind: kinds[next(4) as usize] });
                        }
                        4 if !new.walls.is_empty() => {
                            let i = next(new.walls.len() as u64) as usize;
                            let wall = &mut new.walls[i];
                            let grow = at(next(9)) - 2.0;
                            if wall.x0 == wall.x1 {
                                wall.z1 = (wall.z1 + grow).max(wall.z0 + 0.5);
                            } else {
                                wall.x1 = (wall.x1 + grow).max(wall.x0 + 0.5);
                            }
                            wall.kind = kinds[next(4) as usize];
                        }
                        5 if !new.walls.is_empty() => {
                            let i = next(new.walls.len() as u64) as usize;
                            let id = new.walls.remove(i).id;
                            new.doors.retain(|x| x.wall != id);
                            new.windows.retain(|x| x.wall != id);
                            new.paint.retain(|x| x.wall != id);
                        }
                        6 if !new.walls.is_empty() => {
                            let i = next(new.walls.len() as u64) as usize;
                            new.paint.push(WallPaint { wall: new.walls[i].id.clone(), side: 1, paint: format!("#{:06x}", next(3)) });
                        }
                        7 => new.floors.rects.push(FloorRect { x: 0.0, z: 0.0, w: at(1 + next(8)), d: at(1 + next(8)), m: "oak".into() }),
                        _ if !new.floors.rects.is_empty() => {
                            new.floors.rects.pop();
                        }
                        _ => {}
                    }
                    let now = crate::lot::ids(&new);
                    new.gift = lot.gift.iter().filter(|id| now.contains(*id)).cloned().collect();
                    paid += diff(&lot, &new);
                    lot = new;
                    // Selling what is left can't give back more than went in.
                    assert!(sell_value(&lot) <= paid, "{start} (gift {gifted}), step {step}: {} back for {paid} paid", sell_value(&lot));
                }
                // And selling it all, one way or the other, agrees.
                let mut cleared = lot.clone();
                (cleared.walls, cleared.doors, cleared.windows, cleared.furniture, cleared.storage, cleared.paint) = (vec![], vec![], vec![], vec![], vec![], vec![]);
                cleared.floors.rects.clear();
                cleared.floors.default = table().bare_floor.clone();
                assert_eq!(-diff(&lot, &cleared), sell_value(&lot));
            }
        }
    }
}
