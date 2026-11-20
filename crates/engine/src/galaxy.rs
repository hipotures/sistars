use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::types::Wormhole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GalaxySize {
    Small,
    Medium,
    Large,
    Huge,
    Colossal,
    Galactic,
}

impl GalaxySize {
    pub const fn specs(self) -> (usize, f32, f32) {
        // (planet_count, world_w_ly, world_h_ly) — ~5,000-5,400 sq ly per planet (Stars! standard)
        match self {
            GalaxySize::Small => (120, 1000.0, 650.0),
            GalaxySize::Medium => (250, 1450.0, 950.0),
            GalaxySize::Large => (500, 2000.0, 1350.0),
            GalaxySize::Huge => (1000, 2800.0, 1900.0),
            GalaxySize::Colossal => (10_000, 8800.0, 6000.0),
            GalaxySize::Galactic => (100_000, 28_000.0, 19_000.0),
        }
    }

    pub fn from_str_code(s: &str) -> Option<Self> {
        match s {
            "small" => Some(GalaxySize::Small),
            "medium" => Some(GalaxySize::Medium),
            "large" => Some(GalaxySize::Large),
            "huge" => Some(GalaxySize::Huge),
            "colossal" => Some(GalaxySize::Colossal),
            "galactic" => Some(GalaxySize::Galactic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GalaxyShape {
    Spiral,
    Random,
    Ring,
    Clusters,
    Elliptical,
}

impl GalaxyShape {
    pub fn from_str_code(s: &str) -> Option<Self> {
        match s {
            "spiral" => Some(GalaxyShape::Spiral),
            "random" => Some(GalaxyShape::Random),
            "ring" => Some(GalaxyShape::Ring),
            "clusters" => Some(GalaxyShape::Clusters),
            "elliptical" => Some(GalaxyShape::Elliptical),
            _ => None,
        }
    }
}

pub struct GeneratedGalaxy {
    pub world_w: f32,
    pub world_h: f32,
    pub pos_x: Vec<f32>,
    pub pos_y: Vec<f32>,
    pub hab: Vec<f32>,
    pub min_crust: Vec<f32>,
    pub wormholes: Vec<Wormhole>,
}

fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed as f64 / 4294967296.0) as f32
}

/// Approximate standard normal sample using Irwin-Hall (sum of 4 uniforms - 2).
fn rand_normal(seed: &mut u32) -> f32 {
    (next_rand(seed) + next_rand(seed) + next_rand(seed) + next_rand(seed) - 2.0) * 0.85
}

struct SpacingGrid {
    cols: usize,
    rows: usize,
    inv_cell: f32,
    head: Vec<i32>,
    next: Vec<i32>,
}

impl SpacingGrid {
    fn new(world_w: f32, world_h: f32, cell_size: f32, capacity: usize) -> Self {
        let inv_cell = 1.0 / cell_size;
        let cols = ((world_w * inv_cell).ceil() as usize).max(1);
        let rows = ((world_h * inv_cell).ceil() as usize).max(1);
        Self {
            cols,
            rows,
            inv_cell,
            head: vec![-1; cols * rows],
            next: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        let cx = ((x * self.inv_cell) as isize).clamp(0, (self.cols - 1) as isize) as usize;
        let cy = ((y * self.inv_cell) as isize).clamp(0, (self.rows - 1) as isize) as usize;
        (cx, cy)
    }

    #[inline]
    fn far_enough(&self, x: f32, y: f32, xs: &[f32], ys: &[f32], min_dist_sq: f32) -> bool {
        let (cx, cy) = self.cell(x, y);
        let min_x = cx.saturating_sub(1);
        let max_x = (cx + 1).min(self.cols - 1);
        let min_y = cy.saturating_sub(1);
        let max_y = (cy + 1).min(self.rows - 1);
        for gy in min_y..=max_y {
            let row_off = gy * self.cols;
            for gx in min_x..=max_x {
                let mut cur = self.head[row_off + gx];
                while cur >= 0 {
                    let i = cur as usize;
                    let dx = xs[i] - x;
                    let dy = ys[i] - y;
                    if dx * dx + dy * dy < min_dist_sq {
                        return false;
                    }
                    cur = self.next[i];
                }
            }
        }
        true
    }

    #[inline]
    fn has_local_neighbors(
        &self,
        x: f32,
        y: f32,
        xs: &[f32],
        ys: &[f32],
        radius: f32,
        min_neighbors: usize,
    ) -> bool {
        let r_cells = (radius * self.inv_cell).ceil() as usize;
        let radius_sq = radius * radius;
        let (cx, cy) = self.cell(x, y);
        let min_x = cx.saturating_sub(r_cells);
        let max_x = (cx + r_cells).min(self.cols - 1);
        let min_y = cy.saturating_sub(r_cells);
        let max_y = (cy + r_cells).min(self.rows - 1);
        let mut count = 0usize;
        for gy in min_y..=max_y {
            let row_off = gy * self.cols;
            for gx in min_x..=max_x {
                let mut cur = self.head[row_off + gx];
                while cur >= 0 {
                    let i = cur as usize;
                    let dx = xs[i] - x;
                    let dy = ys[i] - y;
                    let d_sq = dx * dx + dy * dy;
                    if d_sq > 1.0 && d_sq <= radius_sq {
                        count += 1;
                        if count >= min_neighbors {
                            return true;
                        }
                    }
                    cur = self.next[i];
                }
            }
        }
        false
    }

    #[inline]
    fn insert(&mut self, idx: usize, x: f32, y: f32) {
        let (cx, cy) = self.cell(x, y);
        let cell = cy * self.cols + cx;
        let prev = self.head[cell];
        if self.next.len() <= idx {
            self.next.resize(idx + 1, -1);
        }
        self.next[idx] = prev;
        self.head[cell] = idx as i32;
    }
}

pub fn generate_galaxy(
    size: GalaxySize,
    shape: GalaxyShape,
    num_empires: usize,
    crust_richness: f32,
    wormhole_pairs: usize,
    rng_seed: &mut u32,
) -> GeneratedGalaxy {
    let (target_planets, world_w, world_h) = size.specs();
    let n = target_planets.max(num_empires + 12);
    let margin = 45.0f32;
    let cx = world_w * 0.5;
    let cy = world_h * 0.5;
    let rx = world_w * 0.44;
    let ry = world_h * 0.43;
    let min_spacing = 18.0f32;
    let min_spacing_sq = min_spacing * min_spacing;

    let mut pos_x = Vec::with_capacity(n);
    let mut pos_y = Vec::with_capacity(n);
    let mut grid = SpacingGrid::new(world_w, world_h, min_spacing, n);

    let base_angle = next_rand(rng_seed) * TAU;
    let mut cluster_centers: Vec<(f32, f32)> = Vec::new();
    if shape == GalaxyShape::Clusters {
        let total_clusters = (num_empires + (n / 65).clamp(2, 16)).max(num_empires);
        for k in 0..total_clusters {
            let ang = base_angle + (k as f32 / total_clusters as f32) * TAU;
            let rad_mod = if k < num_empires {
                0.72 + (k % 2) as f32 * 0.14
            } else {
                0.35 + next_rand(rng_seed) * 0.25
            };
            let x = (cx + ang.cos() * (rx * rad_mod)).clamp(margin + 50.0, world_w - margin - 50.0);
            let y = (cy + ang.sin() * (ry * rad_mod)).clamp(margin + 50.0, world_h - margin - 50.0);
            cluster_centers.push((x, y));
        }
    }

    // 1. Generate all N stars strictly inside the chosen galaxy topology (O(N) via SpacingGrid)
    let num_arms = if n >= 400 { 4 } else { 3 };
    let arm_twist = 2.6f32;
    let mut safety = 0usize;
    let max_attempts = n.saturating_mul(10).max(6000);

    while pos_x.len() < n && safety < max_attempts {
        safety += 1;
        let (x, y) = match shape {
            GalaxyShape::Random => (
                margin + next_rand(rng_seed) * (world_w - margin * 2.0),
                margin + next_rand(rng_seed) * (world_h - margin * 2.0),
            ),
            GalaxyShape::Spiral => {
                if next_rand(rng_seed) < 0.14 {
                    let r = next_rand(rng_seed).powf(0.7) * 0.24;
                    let a = next_rand(rng_seed) * TAU;
                    (cx + a.cos() * rx * r, cy + a.sin() * ry * r)
                } else {
                    let arm = (next_rand(rng_seed) * num_arms as f32) as usize % num_arms;
                    let r = 0.12 + next_rand(rng_seed).powf(0.75) * 0.88;
                    let arm_base = base_angle + (arm as f32 / num_arms as f32) * TAU;
                    let scatter = rand_normal(rng_seed) * (0.16 + 0.14 * (1.0 - r));
                    let theta = arm_base + r * arm_twist + scatter;
                    let radial_jitter = rand_normal(rng_seed) * 0.05;
                    let rr = (r + radial_jitter).clamp(0.05, 1.0);
                    (cx + theta.cos() * rx * rr, cy + theta.sin() * ry * rr)
                }
            }
            GalaxyShape::Ring => {
                let r = (0.58 + rand_normal(rng_seed) * 0.16).clamp(0.40, 1.0);
                let theta = next_rand(rng_seed) * TAU;
                (cx + theta.cos() * rx * r, cy + theta.sin() * ry * r)
            }
            GalaxyShape::Clusters => {
                let c_idx = (next_rand(rng_seed) * cluster_centers.len() as f32) as usize
                    % cluster_centers.len();
                let (ccx, ccy) = cluster_centers[c_idx];
                let spread = (world_w.min(world_h) * 0.13).clamp(70.0, 1400.0);
                (
                    ccx + rand_normal(rng_seed) * spread,
                    ccy + rand_normal(rng_seed) * spread,
                )
            }
            GalaxyShape::Elliptical => {
                let r = next_rand(rng_seed).powf(0.62);
                let theta = next_rand(rng_seed) * TAU;
                (cx + theta.cos() * rx * r, cy + theta.sin() * ry * r)
            }
        };

        if x < margin || x > world_w - margin || y < margin || y > world_h - margin {
            continue;
        }
        if grid.far_enough(x, y, &pos_x, &pos_y, min_spacing_sq) {
            let idx = pos_x.len();
            pos_x.push(x);
            pos_y.push(y);
            grid.insert(idx, x, y);
        }
    }

    // Fallback fill if rejection sampling hit safety limit
    while pos_x.len() < n {
        pos_x.push(margin + next_rand(rng_seed) * (world_w - margin * 2.0));
        pos_y.push(margin + next_rand(rng_seed) * (world_h - margin * 2.0));
    }

    // 2. Select homeworlds FROM ACTUAL GENERATED STARS using Farthest-Point Sampling
    //    over well-connected stars (at least 3 neighbors within 110 ly), so every empire
    //    spawns strictly inside the galactic structure (spiral arms, ring, clusters)
    //    and never on a lone statistical outlier!
    let sample_step = (n / 1500).max(1);
    let offset = (next_rand(rng_seed) * sample_step as f32) as usize % n;
    let mut candidates: Vec<usize> = Vec::with_capacity(1500);
    let mut idx = offset;
    while idx < n {
        if grid.has_local_neighbors(pos_x[idx], pos_y[idx], &pos_x, &pos_y, 110.0, 3) {
            candidates.push(idx);
        }
        idx += sample_step;
    }
    // Fallback if a sparse custom map had too few dense candidates
    if candidates.len() < num_empires {
        candidates.clear();
        let mut i = offset;
        while i < n {
            candidates.push(i);
            i += sample_step;
        }
        for i in 0..n {
            if candidates.len() >= num_empires {
                break;
            }
            if !candidates.contains(&i) {
                candidates.push(i);
            }
        }
    }

    let mut chosen: Vec<usize> = Vec::with_capacity(num_empires);

    // Pick first homeworld from the outer half of the structure
    let mut first_idx = candidates[0];
    let mut best_first_r = -1.0f32;
    for &i in &candidates {
        let nx = (pos_x[i] - cx) / rx;
        let ny = (pos_y[i] - cy) / ry;
        let r = (nx * nx + ny * ny).sqrt();
        if r >= 0.45 && r <= 0.85 && r > best_first_r {
            best_first_r = r;
            first_idx = i;
        }
    }
    chosen.push(first_idx);

    for _ in 1..num_empires {
        let mut best_cand = candidates[0];
        let mut best_min_dist_sq = -1.0f32;
        for &c_idx in &candidates {
            if chosen.contains(&c_idx) {
                continue;
            }
            let x = pos_x[c_idx];
            let y = pos_y[c_idx];
            let mut min_d_sq = f32::MAX;
            for &c in &chosen {
                let dx = pos_x[c] - x;
                let dy = pos_y[c] - y;
                let d_sq = dx * dx + dy * dy;
                if d_sq < min_d_sq {
                    min_d_sq = d_sq;
                }
            }
            if min_d_sq > best_min_dist_sq {
                best_min_dist_sq = min_d_sq;
                best_cand = c_idx;
            }
        }
        chosen.push(best_cand);
    }

    // Swap chosen homeworlds into indices 0..num_empires
    for e in 0..num_empires {
        let target = chosen[e];
        if target != e {
            pos_x.swap(e, target);
            pos_y.swap(e, target);
            for c in chosen.iter_mut().skip(e + 1) {
                if *c == e {
                    *c = target;
                }
            }
        }
    }

    let mut hab = vec![0.0f32; n];
    let mut min_crust = vec![0.0f32; n * 3];
    for i in 0..n {
        hab[i] = (0.25 + next_rand(rng_seed).powf(1.3) * 0.75).clamp(0.25, 1.0);
        for m in 0..3 {
            min_crust[i * 3 + m] =
                crust_richness * (0.2 + next_rand(rng_seed).powf(1.4) * 1.5);
        }
    }

    // 4. Wormhole pairs bridging opposite halves of the galaxy
    let mut wormholes = Vec::with_capacity(wormhole_pairs);
    for _ in 0..wormhole_pairs {
        let x1 = world_w * (0.10 + next_rand(rng_seed) * 0.32);
        let y1 = world_h * (0.14 + next_rand(rng_seed) * 0.72);
        let x2 = world_w * (0.58 + next_rand(rng_seed) * 0.32);
        let y2 = world_h * (0.14 + next_rand(rng_seed) * 0.72);
        wormholes.push(Wormhole { x1, y1, x2, y2 });
    }

    GeneratedGalaxy {
        world_w,
        world_h,
        pos_x,
        pos_y,
        hab,
        min_crust,
        wormholes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_all_sizes_and_shapes_cleanly() {
        for size in [
            GalaxySize::Small,
            GalaxySize::Medium,
            GalaxySize::Large,
            GalaxySize::Huge,
        ] {
            let (expected_n, exp_w, exp_h) = size.specs();
            for shape in [
                GalaxyShape::Spiral,
                GalaxyShape::Random,
                GalaxyShape::Ring,
                GalaxyShape::Clusters,
                GalaxyShape::Elliptical,
            ] {
                let mut seed = 12345u32;
                let g = generate_galaxy(size, shape, 6, 800.0, 2, &mut seed);
                assert_eq!(g.pos_x.len(), expected_n);
                assert_eq!(g.world_w, exp_w);
                assert_eq!(g.world_h, exp_h);
            }
        }
    }

    #[test]
    fn homeworlds_spawn_inside_structure_with_neighbors() {
        for size in [GalaxySize::Huge, GalaxySize::Colossal] {
            for shape in [GalaxyShape::Spiral, GalaxyShape::Ring, GalaxyShape::Clusters] {
                for seed_init in [42u32, 1337, 99999] {
                    let mut seed = seed_init;
                    let num_empires = 6;
                    let g = generate_galaxy(size, shape, num_empires, 800.0, 2, &mut seed);
                    for e in 0..num_empires {
                        let hx = g.pos_x[e];
                        let hy = g.pos_y[e];
                        let nearby = (0..g.pos_x.len())
                            .filter(|&j| {
                                j != e
                                    && (g.pos_x[j] - hx).hypot(g.pos_y[j] - hy) <= 115.0
                            })
                            .count();
                        assert!(
                            nearby >= 3,
                            "Empire {e} on {size:?}/{shape:?} (seed {seed_init}) has only {nearby} neighbors within 115 ly!"
                        );
                    }
                }
            }
        }
    }
}
