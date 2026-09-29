//! A 64 × 64 tileable blue-noise dither table, generated once with Ulichney's void-and-cluster
//! method.
//!
//! The generation is deterministic on every platform: a fixed-seed PRNG for the initial pattern
//! and a Gaussian kernel built from one constant with basic (correctly rounded) arithmetic only,
//! so that exports are identical everywhere.

use std::sync::OnceLock;

/// Edge of the (toroidal) table.
pub(crate) const SIZE: usize = 64;
const COUNT: usize = SIZE * SIZE;

/// Amplitude of the offsets: strictly below half a step, so a value exactly on a level never
/// moves.
const AMPLITUDE: f32 = 0.49;

/// `exp(-1 / (2σ²))` for σ = 1.5, the usual void-and-cluster kernel width.
const KERNEL_BASE: f64 = 0.800_737_402_916_808_1;

/// Dither offsets for row `y`, indexed by `x % SIZE`: uniformly spread in `[-0.49, 0.49]`
/// (in steps of the output), with blue-noise (high-frequency only) spatial structure.
pub(crate) fn row(y: u64) -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        ranks()
            .into_iter()
            .map(|rank| ((f32::from(rank) + 0.5) / COUNT as f32 - 0.5) * (2.0 * AMPLITUDE))
            .collect()
    });
    let start = (y % SIZE as u64) as usize * SIZE;
    &table[start..start + SIZE]
}

/// Void-and-cluster ranks: a permutation of `0..COUNT`, row-major.
fn ranks() -> Vec<u16> {
    let kernel = kernel();
    let mut prototype = Pattern::new(&kernel);
    // Initial pattern: 10 % of the pixels, at random.
    let ones = COUNT / 10;
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    while prototype.ones < ones {
        let p = (rng.next() % COUNT as u64) as usize;
        if !prototype.on[p] {
            prototype.toggle(p);
        }
    }
    // Relax it: move the tightest cluster into the largest void until that changes nothing.
    // Bounded for safety; it converges in far fewer steps.
    for _ in 0..COUNT {
        let cluster = prototype.tightest_cluster();
        prototype.toggle(cluster);
        let void = prototype.largest_void();
        prototype.toggle(void);
        if void == cluster {
            break;
        }
    }

    let mut ranks = vec![0u16; COUNT];
    // Phase 1: remove the tightest clusters of the prototype, ranking them downwards.
    let mut pattern = prototype.clone();
    for rank in (0..ones).rev() {
        let cluster = pattern.tightest_cluster();
        pattern.toggle(cluster);
        ranks[cluster] = rank as u16;
    }
    // Phases 2 and 3: fill the largest voids, ranking them upwards. (Past half, Ulichney picks
    // the tightest cluster of zeros; the kernel sums to the same total everywhere, so that is
    // the same pixel as the largest void of ones.)
    let mut pattern = prototype;
    for rank in ones..COUNT {
        let void = pattern.largest_void();
        pattern.toggle(void);
        ranks[void] = rank as u16;
    }
    ranks
}

/// Toroidal Gaussian kernel, indexed by `dy * SIZE + dx`.
fn kernel() -> Vec<f64> {
    let half = SIZE / 2;
    let max_d2 = 2 * half * half;
    let mut powers = Vec::with_capacity(max_d2 + 1);
    let mut power = 1.0;
    for _ in 0..=max_d2 {
        powers.push(power);
        power *= KERNEL_BASE;
    }
    (0..COUNT)
        .map(|i| {
            let (dx, dy) = (i % SIZE, i / SIZE);
            let (dx, dy) = (dx.min(SIZE - dx), dy.min(SIZE - dy));
            powers[dx * dx + dy * dy]
        })
        .collect()
}

#[derive(Clone)]
struct Pattern<'a> {
    kernel: &'a [f64],
    on: Vec<bool>,
    ones: usize,
    /// Sum of the kernel centered on every "on" pixel, at each pixel.
    energy: Vec<f64>,
}

impl<'a> Pattern<'a> {
    fn new(kernel: &'a [f64]) -> Self {
        Self {
            kernel,
            on: vec![false; COUNT],
            ones: 0,
            energy: vec![0.0; COUNT],
        }
    }

    fn toggle(&mut self, p: usize) {
        let sign = if self.on[p] { -1.0 } else { 1.0 };
        self.on[p] = !self.on[p];
        if self.on[p] {
            self.ones += 1;
        } else {
            self.ones -= 1;
        }
        let (px, py) = (p % SIZE, p / SIZE);
        for (q, energy) in self.energy.iter_mut().enumerate() {
            let dx = (q % SIZE + SIZE - px) % SIZE;
            let dy = (q / SIZE + SIZE - py) % SIZE;
            *energy += sign * self.kernel[dy * SIZE + dx];
        }
    }

    /// The "on" pixel with the highest energy (the first one on ties).
    fn tightest_cluster(&self) -> usize {
        self.extreme(true, |candidate, best| candidate > best)
    }

    /// The "off" pixel with the lowest energy (the first one on ties).
    fn largest_void(&self) -> usize {
        self.extreme(false, |candidate, best| candidate < best)
    }

    fn extreme(&self, on: bool, better: impl Fn(f64, f64) -> bool) -> usize {
        let mut best: Option<usize> = None;
        for p in (0..COUNT).filter(|&p| self.on[p] == on) {
            if best.is_none_or(|b| better(self.energy[p], self.energy[b])) {
                best = Some(p);
            }
        }
        // Callers only ask while both kinds of pixel exist; 0 is a harmless fallback.
        best.unwrap_or(0)
    }
}

/// Marsaglia's xorshift64: tiny, deterministic, good enough for an initial pattern.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_a_permutation() {
        let mut ranks = ranks();
        ranks.sort_unstable();
        assert!(ranks.iter().enumerate().all(|(i, &r)| usize::from(r) == i));
    }

    #[test]
    fn offsets_are_bounded_and_uniform() {
        let offsets: Vec<f32> = (0..SIZE as u64).flat_map(|y| row(y).to_vec()).collect();
        assert!(offsets.iter().all(|n| n.abs() < 0.49 + 1e-6));
        let mean = offsets.iter().map(|&n| f64::from(n)).sum::<f64>() / COUNT as f64;
        assert!(mean.abs() < 1e-6, "{mean}");
        // Rows wrap around.
        assert_eq!(row(3), row(3 + SIZE as u64));
    }

    #[test]
    fn low_ranks_are_well_spread() {
        // Blue noise: the first 10 % of the ranks (a sparse dot pattern) has no two dots
        // touching, even diagonally, unlike white noise.
        let ranks = ranks();
        let dots: Vec<usize> = (0..COUNT)
            .filter(|&p| usize::from(ranks[p]) < COUNT / 10)
            .collect();
        for &a in &dots {
            for &b in &dots {
                if a == b {
                    continue;
                }
                let dx = (a % SIZE).abs_diff(b % SIZE);
                let dy = (a / SIZE).abs_diff(b / SIZE);
                let (dx, dy) = (dx.min(SIZE - dx), dy.min(SIZE - dy));
                assert!(dx * dx + dy * dy > 2, "dots {a} and {b} touch");
            }
        }
    }
}
