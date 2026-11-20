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
}

impl GalaxySize {
    pub const fn specs(self) -> (usize, f32, f32) {
        // (planet_count, world_w_ly, world_h_ly) — ~5,000-5,400 sq ly per planet (Stars! standard)
        match self {
            GalaxySize::Small => (120, 1000.0, 650.0),
            GalaxySize::Medium => (250, 1450.0, 950.0),
            GalaxySize::Large => (500, 2000.0, 1350.0),
            GalaxySize::Huge => (1000, 2800.0, 1900.0),
        }
    }

    pub fn from_str_code(s: &str) -> Option<Self> {
        match s {
            "small" => Some(GalaxySize::Small),
            "medium" => Some(GalaxySize::Medium),
            "large" => Some(GalaxySize::Large),
            "huge" => Some(GalaxySize::Huge),
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

fn far_enough(x: f32, y: f32, xs: &[f32], ys: &[f32], min_dist_sq: f32) -> bool {
    for i in 0..xs.len() {
        let dx = xs[i] - x;
        let dy = ys[i] - y;
        if dx * dx + dy * dy < min_dist_sq {
            return false;
        }
    }
    true
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

    // 1. Place homeworlds (indices 0..num_empires) symmetrically according to galaxy shape
    let base_angle = next_rand(rng_seed) * TAU;
    let mut cluster_centers: Vec<(f32, f32)> = Vec::new();

    for e in 0..num_empires {
        let frac = (e as f32) / (num_empires as f32);
        let ang = base_angle + frac * TAU;
        let (hx, hy) = match shape {
            GalaxyShape::Ring => (cx + ang.cos() * (rx * 0.82), cy + ang.sin() * (ry * 0.82)),
            GalaxyShape::Clusters => {
                let rad_mod = 0.72 + (e % 2) as f32 * 0.14;
                let x = (cx + ang.cos() * (rx * rad_mod)).clamp(margin + 50.0, world_w - margin - 50.0);
                let y = (cy + ang.sin() * (ry * rad_mod)).clamp(margin + 50.0, world_h - margin - 50.0);
                cluster_centers.push((x, y));
                (x, y)
            }
            GalaxyShape::Spiral => {
                let x = cx + ang.cos() * (rx * 0.72);
                let y = cy + ang.sin() * (ry * 0.72);
                (x, y)
            }
            GalaxyShape::Elliptical | GalaxyShape::Random => {
                (cx + ang.cos() * (rx * 0.76), cy + ang.sin() * (ry * 0.76))
            }
        };
        pos_x.push(hx.clamp(margin, world_w - margin));
        pos_y.push(hy.clamp(margin, world_h - margin));
    }

    // Add extra neutral cluster centers for Clusters shape
    if shape == GalaxyShape::Clusters {
        cluster_centers.push((cx, cy));
        let extra = (n / 65).clamp(2, 6);
        for k in 0..extra {
            let ang = base_angle + 0.5 + (k as f32 / extra as f32) * TAU;
            let x = (cx + ang.cos() * (rx * 0.42)).clamp(margin + 40.0, world_w - margin - 40.0);
            let y = (cy + ang.sin() * (ry * 0.42)).clamp(margin + 40.0, world_h - margin - 40.0);
            cluster_centers.push((x, y));
        }
    }

    // 2. Guarantee 2 local frontier worlds within 45..78 ly of each homeworld so no empire is ever stranded
    for e in 0..num_empires {
        let hx = pos_x[e];
        let hy = pos_y[e];
        for _ in 0..2 {
            if pos_x.len() >= n {
                break;
            }
            for _attempt in 0..25 {
                let a = next_rand(rng_seed) * TAU;
                let d = 42.0 + next_rand(rng_seed) * 34.0;
                let x = (hx + a.cos() * d).clamp(margin, world_w - margin);
                let y = (hy + a.sin() * d).clamp(margin, world_h - margin);
                if far_enough(x, y, &pos_x, &pos_y, min_spacing_sq) {
                    pos_x.push(x);
                    pos_y.push(y);
                    break;
                }
            }
        }
    }

    // 3. Generate remaining stars using chosen galaxy topology
    let num_arms = if n >= 400 { 4 } else { 3 };
    let arm_twist = 2.6f32;
    let mut safety = 0usize;

    while pos_x.len() < n && safety < n * 40 {
        safety += 1;
        let (x, y) = match shape {
            GalaxyShape::Random => (
                margin + next_rand(rng_seed) * (world_w - margin * 2.0),
                margin + next_rand(rng_seed) * (world_h - margin * 2.0),
            ),
            GalaxyShape::Spiral => {
                if next_rand(rng_seed) < 0.14 {
                    // Galactic core bulge
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
                let spread = (world_w.min(world_h) * 0.13).clamp(70.0, 180.0);
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
        if far_enough(x, y, &pos_x, &pos_y, min_spacing_sq) {
            pos_x.push(x);
            pos_y.push(y);
        }
    }

    // Fallback fill if rejection sampling hit safety limit
    while pos_x.len() < n {
        pos_x.push(margin + next_rand(rng_seed) * (world_w - margin * 2.0));
        pos_y.push(margin + next_rand(rng_seed) * (world_h - margin * 2.0));
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
}
