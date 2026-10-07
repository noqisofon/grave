use crate::rng::Rng;

pub const W: i32 = 70;
pub const H: i32 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Wall,
    Floor,
    Stairs,
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Tile::Wall => '#',
            Tile::Floor => '.',
            Tile::Stairs => '>',
        }
    }

    pub fn walkable(self) -> bool {
        self != Tile::Wall
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// 壁1枚ぶんの余白を含めて重なるか。
    fn overlaps(&self, o: &Rect) -> bool {
        self.x - 1 < o.x + o.w
            && o.x < self.x + self.w + 1
            && self.y - 1 < o.y + o.h
            && o.y < self.y + self.h + 1
    }
}

pub struct Map {
    tiles: Vec<Tile>,
    seen: Vec<bool>,
    visible: Vec<bool>,
}

pub struct Generated {
    pub map: Map,
    pub start: (i32, i32),
    pub stairs: (i32, i32),
}

pub fn idx(x: i32, y: i32) -> usize {
    (y * W + x) as usize
}

impl Map {
    pub fn in_bounds(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < W && y < H
    }

    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if Self::in_bounds(x, y) {
            self.tiles[idx(x, y)]
        } else {
            Tile::Wall
        }
    }

    pub fn is_seen(&self, x: i32, y: i32) -> bool {
        Self::in_bounds(x, y) && self.seen[idx(x, y)]
    }

    pub fn is_visible(&self, x: i32, y: i32) -> bool {
        Self::in_bounds(x, y) && self.visible[idx(x, y)]
    }

    pub fn generate(rng: &mut Rng) -> Generated {
        let mut tiles = vec![Tile::Wall; (W * H) as usize];
        let mut rooms: Vec<Rect> = Vec::new();

        for _ in 0..300 {
            if rooms.len() >= 8 {
                break;
            }
            let w = rng.range(4, 12);
            let h = rng.range(3, 7);
            let x = rng.range(1, W - w - 1);
            let y = rng.range(1, H - h - 1);
            let r = Rect { x, y, w, h };
            if rooms.iter().any(|o| o.overlaps(&r)) {
                continue;
            }
            for yy in r.y..r.y + r.h {
                for xx in r.x..r.x + r.w {
                    tiles[idx(xx, yy)] = Tile::Floor;
                }
            }
            if let Some(prev) = rooms.last() {
                let (ax, ay) = prev.center();
                let (bx, by) = r.center();
                if rng.range(0, 2) == 0 {
                    carve_h(&mut tiles, ax, bx, ay);
                    carve_v(&mut tiles, ay, by, bx);
                } else {
                    carve_v(&mut tiles, ay, by, ax);
                    carve_h(&mut tiles, ax, bx, by);
                }
            }
            rooms.push(r);
        }

        let start = rooms[0].center();
        let stairs = if rooms.len() > 1 {
            rooms[rooms.len() - 1].center()
        } else {
            (start.0 + 1, start.1)
        };
        tiles[idx(stairs.0, stairs.1)] = Tile::Stairs;

        Generated {
            map: Map {
                tiles,
                seen: vec![false; (W * H) as usize],
                visible: vec![false; (W * H) as usize],
            },
            start,
            stairs,
        }
    }

    /// bresenham で a から b まで壁に遮られず見通せるか。壁そのものは見える。
    pub fn los(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        let (mut x, mut y) = a;
        let dx = (b.0 - a.0).abs();
        let dy = -(b.1 - a.1).abs();
        let sx = if a.0 < b.0 { 1 } else { -1 };
        let sy = if a.1 < b.1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            if (x, y) == b {
                return true;
            }
            if (x, y) != a && self.tile(x, y) == Tile::Wall {
                return false;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// 歩ける場所と、それに接する壁を既知にする（地図の巻物）。
    pub fn reveal_all(&mut self) {
        for y in 0..H {
            for x in 0..W {
                if self.tile(x, y).walkable() {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if Self::in_bounds(x + dx, y + dy) {
                                self.seen[idx(x + dx, y + dy)] = true;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_fov(&mut self, pos: (i32, i32), radius: i32) {
        self.visible.iter_mut().for_each(|v| *v = false);
        for y in pos.1 - radius..=pos.1 + radius {
            for x in pos.0 - radius..=pos.0 + radius {
                if !Self::in_bounds(x, y) {
                    continue;
                }
                let (dx, dy) = (x - pos.0, y - pos.1);
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                if self.los(pos, (x, y)) {
                    let i = idx(x, y);
                    self.visible[i] = true;
                    self.seen[i] = true;
                }
            }
        }
    }
}

fn carve_h(tiles: &mut [Tile], x1: i32, x2: i32, y: i32) {
    for x in x1.min(x2)..=x1.max(x2) {
        tiles[idx(x, y)] = Tile::Floor;
    }
}

fn carve_v(tiles: &mut [Tile], y1: i32, y2: i32, x: i32) {
    for y in y1.min(y2)..=y1.max(y2) {
        tiles[idx(x, y)] = Tile::Floor;
    }
}
