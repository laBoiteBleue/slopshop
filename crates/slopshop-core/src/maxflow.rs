//! Minimum cut on a pixel grid: each pixel linked to its 8 neighbors and to two terminals (the
//! source, "inside", and the sink, "outside"). Solved by Boykov and Kolmogorov's max-flow
//! ("An Experimental Comparison of Min-Cut/Max-Flow Algorithms for Energy Minimization in
//! Vision", 2004): two search trees grown from the terminals, augmenting paths where they meet,
//! orphans adopted again; fast on the short paths of image grids. Used by Quick Selection.

use std::collections::VecDeque;

/// Neighbor offsets; direction `k`'s opposite is `k ^ 1`.
const DIRS: [(isize, isize); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
];

/// Number of neighbors of a pixel.
pub const NEIGHBORS: usize = 8;

const FREE: u8 = 0;
const SOURCE: u8 = 1;
const SINK: u8 = 2;

/// `parent` values besides a direction (0–7).
const TERMINAL: u8 = 8;
const ORPHAN: u8 = 9;
const NONE: u8 = 10;

const INFINITE_DISTANCE: u32 = u32::MAX;

/// Capacity left below which a link counts as saturated: negligible links (two very different
/// neighbors weigh about 1e-20) would otherwise cost countless augmentations of nothing.
const EPSILON: f32 = 1.0e-3;

/// A grid graph to cut: capacities between neighbors and to the terminals.
pub struct Grid {
    width: usize,
    height: usize,
    /// Residual capacity from each pixel to its neighbor in each direction (`p × 8 + k`).
    capacity: Vec<f32>,
    /// Net residual capacity to the terminals: > 0 from the source, < 0 to the sink.
    terminal: Vec<f32>,
}

impl Grid {
    /// A `width × height` grid without capacities.
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            capacity: vec![0.0; n * NEIGHBORS],
            terminal: vec![0.0; n],
        }
    }

    /// The neighbor of pixel `p` in direction `k`, if inside the grid.
    pub fn neighbor(&self, p: usize, k: usize) -> Option<usize> {
        let (dx, dy) = DIRS[k];
        let x = (p % self.width).checked_add_signed(dx)?;
        let y = (p / self.width).checked_add_signed(dy)?;
        (x < self.width && y < self.height).then_some(y * self.width + x)
    }

    /// The offset of direction `k`, pixels.
    pub fn direction(k: usize) -> (isize, isize) {
        DIRS[k]
    }

    /// The cost of cutting `p` from the source (it ends outside) and from the sink (inside).
    /// Only their difference matters to the cut.
    pub fn set_terminals(&mut self, p: usize, source: f32, sink: f32) {
        self.terminal[p] = source - sink;
    }

    /// The capacity from `p` to its neighbor in direction `k` (and none back: set that one from
    /// the neighbor).
    pub fn set_capacity(&mut self, p: usize, k: usize, capacity: f32) {
        self.capacity[p * NEIGHBORS + k] = capacity;
    }

    /// The minimum cut: for each pixel, whether it stays connected to the source (inside).
    pub fn solve(mut self) -> Vec<bool> {
        let n = self.width * self.height;
        let mut tree = vec![FREE; n];
        let mut parent = vec![NONE; n];
        let mut stamp = vec![0u32; n];
        let mut distance = vec![0u32; n];
        let mut active: VecDeque<usize> = VecDeque::new();
        let mut is_active = vec![false; n];
        let mut orphans: VecDeque<usize> = VecDeque::new();
        let mut time = 0u32;

        for p in 0..n {
            let t = self.terminal[p];
            if t.abs() > EPSILON {
                tree[p] = if t > 0.0 { SOURCE } else { SINK };
                parent[p] = TERMINAL;
                distance[p] = 1;
                active.push_back(p);
                is_active[p] = true;
            }
        }

        let mut current: Option<usize> = None;
        loop {
            // The active pixel to grow from: the last one while it keeps finding paths.
            let p = match current.filter(|&p| tree[p] != FREE) {
                Some(p) => p,
                None => {
                    let mut next = None;
                    while let Some(p) = active.pop_front() {
                        is_active[p] = false;
                        if tree[p] != FREE {
                            next = Some(p);
                            break;
                        }
                    }
                    match next {
                        Some(p) => p,
                        None => break,
                    }
                }
            };

            // Growth: a path from the source tree to the sink tree, (pixel, pixel, direction).
            let mut path = None;
            for k in 0..NEIGHBORS {
                let Some(q) = self.neighbor(p, k) else {
                    continue;
                };
                let open = if tree[p] == SOURCE {
                    self.capacity[p * NEIGHBORS + k] > EPSILON
                } else {
                    self.capacity[q * NEIGHBORS + (k ^ 1)] > EPSILON
                };
                if !open {
                    continue;
                }
                if tree[q] == FREE {
                    tree[q] = tree[p];
                    parent[q] = (k ^ 1) as u8;
                    stamp[q] = stamp[p];
                    distance[q] = distance[p] + 1;
                    if !is_active[q] {
                        is_active[q] = true;
                        active.push_back(q);
                    }
                } else if tree[q] != tree[p] {
                    path = Some(if tree[p] == SOURCE {
                        (p, q, k)
                    } else {
                        (q, p, k ^ 1)
                    });
                    break;
                } else if stamp[q] <= stamp[p] && distance[q] > distance[p] {
                    // A shorter way to the terminal (a heuristic of the original).
                    parent[q] = (k ^ 1) as u8;
                    stamp[q] = stamp[p];
                    distance[q] = distance[p] + 1;
                }
            }

            let Some((s, t, k)) = path else {
                current = None;
                continue;
            };
            time += 1;
            current = Some(p);
            self.augment(s, t, k, &parent, &mut orphans);
            for &o in &orphans {
                parent[o] = ORPHAN;
            }
            // Adoption.
            while let Some(o) = orphans.pop_front() {
                let side = tree[o];
                let mut best: Option<(usize, u32)> = None;
                for k in 0..NEIGHBORS {
                    let Some(q) = self.neighbor(o, k) else {
                        continue;
                    };
                    if tree[q] != side || !self.residual(side, o, q, k) {
                        continue;
                    }
                    // Is q still rooted at a terminal? Count the way there.
                    let mut j = q;
                    let mut d = 0u32;
                    loop {
                        if stamp[j] == time {
                            d = d.saturating_add(distance[j]);
                            break;
                        }
                        d += 1;
                        match parent[j] {
                            TERMINAL => {
                                stamp[j] = time;
                                distance[j] = 1;
                                break;
                            }
                            ORPHAN | NONE => {
                                d = INFINITE_DISTANCE;
                                break;
                            }
                            dir => {
                                // Invariant: parents are neighbors inside the grid.
                                j = self.neighbor(j, dir as usize).unwrap_or(j);
                            }
                        }
                    }
                    if d == INFINITE_DISTANCE {
                        continue;
                    }
                    if best.is_none_or(|(_, b)| d < b) {
                        best = Some((k, d));
                    }
                    // Remember the distances along the way for the next orphans.
                    let mut j = q;
                    let mut dd = d;
                    while stamp[j] != time {
                        stamp[j] = time;
                        distance[j] = dd;
                        dd = dd.saturating_sub(1);
                        match parent[j] {
                            dir if (dir as usize) < NEIGHBORS => {
                                j = self.neighbor(j, dir as usize).unwrap_or(j);
                            }
                            _ => break,
                        }
                    }
                }
                if let Some((k, d)) = best {
                    parent[o] = k as u8;
                    stamp[o] = time;
                    distance[o] = d + 1;
                    continue;
                }
                // No new parent: free, and its children are orphans in turn.
                tree[o] = FREE;
                parent[o] = NONE;
                for k in 0..NEIGHBORS {
                    let Some(q) = self.neighbor(o, k) else {
                        continue;
                    };
                    if tree[q] != side {
                        continue;
                    }
                    if self.residual(side, o, q, k) && !is_active[q] {
                        is_active[q] = true;
                        active.push_back(q);
                    }
                    let dir = parent[q];
                    if (dir as usize) < NEIGHBORS && self.neighbor(q, dir as usize) == Some(o) {
                        parent[q] = ORPHAN;
                        orphans.push_back(q);
                    }
                }
            }
        }
        tree.into_iter().map(|t| t == SOURCE).collect()
    }

    /// Whether `q` (a neighbor of `o` in direction `k`, in tree `side`) can be its parent: the
    /// edge toward the sink side has capacity left.
    fn residual(&self, side: u8, o: usize, q: usize, k: usize) -> bool {
        if side == SOURCE {
            self.capacity[q * NEIGHBORS + (k ^ 1)] > EPSILON
        } else {
            self.capacity[o * NEIGHBORS + k] > EPSILON
        }
    }

    /// Push the path's bottleneck through it: the source tree's way to `s`, the edge from `s` to
    /// `t` (direction `k`), the sink tree's way from `t`. Pixels whose link to their parent is
    /// saturated become orphans.
    fn augment(
        &mut self,
        s: usize,
        t: usize,
        k: usize,
        parent: &[u8],
        orphans: &mut VecDeque<usize>,
    ) {
        let mut bottleneck = self.capacity[s * NEIGHBORS + k];
        let mut i = s;
        loop {
            match parent[i] {
                TERMINAL => {
                    bottleneck = bottleneck.min(self.terminal[i]);
                    break;
                }
                dir => {
                    let d = dir as usize;
                    let Some(j) = self.neighbor(i, d) else { break };
                    bottleneck = bottleneck.min(self.capacity[j * NEIGHBORS + (d ^ 1)]);
                    i = j;
                }
            }
        }
        let mut i = t;
        loop {
            match parent[i] {
                TERMINAL => {
                    bottleneck = bottleneck.min(-self.terminal[i]);
                    break;
                }
                dir => {
                    let d = dir as usize;
                    let Some(j) = self.neighbor(i, d) else { break };
                    bottleneck = bottleneck.min(self.capacity[i * NEIGHBORS + d]);
                    i = j;
                }
            }
        }

        self.capacity[s * NEIGHBORS + k] -= bottleneck;
        self.capacity[t * NEIGHBORS + (k ^ 1)] += bottleneck;
        let mut i = s;
        loop {
            match parent[i] {
                TERMINAL => {
                    self.terminal[i] -= bottleneck;
                    if self.terminal[i] <= EPSILON {
                        self.terminal[i] = 0.0;
                        orphans.push_back(i);
                    }
                    break;
                }
                dir => {
                    let d = dir as usize;
                    let Some(j) = self.neighbor(i, d) else { break };
                    self.capacity[i * NEIGHBORS + d] += bottleneck;
                    let edge = j * NEIGHBORS + (d ^ 1);
                    self.capacity[edge] -= bottleneck;
                    if self.capacity[edge] <= EPSILON {
                        self.capacity[edge] = 0.0;
                        orphans.push_back(i);
                    }
                    i = j;
                }
            }
        }
        let mut i = t;
        loop {
            match parent[i] {
                TERMINAL => {
                    self.terminal[i] += bottleneck;
                    if self.terminal[i] >= -EPSILON {
                        self.terminal[i] = 0.0;
                        orphans.push_back(i);
                    }
                    break;
                }
                dir => {
                    let d = dir as usize;
                    let Some(j) = self.neighbor(i, d) else { break };
                    self.capacity[j * NEIGHBORS + (d ^ 1)] += bottleneck;
                    let edge = i * NEIGHBORS + d;
                    self.capacity[edge] -= bottleneck;
                    if self.capacity[edge] <= EPSILON {
                        self.capacity[edge] = 0.0;
                        orphans.push_back(i);
                    }
                    i = j;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator for test graphs.
    fn lcg(seed: &mut u64) -> f32 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The cut's cost: terminal links of each side, and edges from inside to outside.
    fn cut_cost(grid: &Grid, sources: &[f32], sinks: &[f32], inside: &[bool]) -> f64 {
        let mut cost = 0.0f64;
        for p in 0..inside.len() {
            cost += f64::from(if inside[p] { sinks[p] } else { sources[p] });
            for k in 0..NEIGHBORS {
                if let Some(q) = grid.neighbor(p, k)
                    && inside[p]
                    && !inside[q]
                {
                    cost += f64::from(grid.capacity[p * NEIGHBORS + k]);
                }
            }
        }
        cost
    }

    #[test]
    fn the_cut_is_minimal_on_small_random_grids() {
        let mut seed = 7u64;
        for case in 0..40 {
            let (w, h) = (2 + case % 3, 2 + case % 2);
            let n = w * h;
            let mut grid = Grid::new(w, h);
            let mut sources = vec![0.0; n];
            let mut sinks = vec![0.0; n];
            for p in 0..n {
                sources[p] = (lcg(&mut seed) * 4.0).floor();
                sinks[p] = (lcg(&mut seed) * 4.0).floor();
                for k in 0..NEIGHBORS {
                    if grid.neighbor(p, k).is_some() {
                        grid.set_capacity(p, k, (lcg(&mut seed) * 3.0).floor());
                    }
                }
            }
            let reference = Grid {
                width: w,
                height: h,
                capacity: grid.capacity.clone(),
                terminal: vec![0.0; n],
            };
            for p in 0..n {
                grid.set_terminals(p, sources[p], sinks[p]);
            }
            let inside = grid.solve();
            // Subtracting min(source, sink) from both does not change which cut is minimal.
            let found = cut_cost(&reference, &sources, &sinks, &inside);
            let best = (0u32..1 << n)
                .map(|bits| {
                    let labels: Vec<bool> = (0..n).map(|p| bits >> p & 1 == 1).collect();
                    cut_cost(&reference, &sources, &sinks, &labels)
                })
                .fold(f64::INFINITY, f64::min);
            assert!((found - best).abs() < 1e-3, "case {case}: {found} > {best}");
        }
    }

    #[test]
    fn a_strong_edge_separates_two_halves() {
        // Left column pulled inside, right column outside, weak links across the middle.
        let (w, h) = (6, 4);
        let mut grid = Grid::new(w, h);
        for p in 0..w * h {
            for k in 0..NEIGHBORS {
                if let Some(q) = grid.neighbor(p, k) {
                    let across = (p % w < 3) != (q % w < 3);
                    grid.set_capacity(p, k, if across { 0.1 } else { 5.0 });
                }
            }
            match p % w {
                0 => grid.set_terminals(p, 100.0, 0.0),
                5 => grid.set_terminals(p, 0.0, 100.0),
                _ => {}
            }
        }
        let inside = grid.solve();
        for (p, &i) in inside.iter().enumerate() {
            assert_eq!(i, p % w < 3, "pixel {p}");
        }
    }
}
