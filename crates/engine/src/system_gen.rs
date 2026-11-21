//! Deterministic, zero-RAM-overhead Dole/Fogg Accrete-style star system generator.
//!
//! Nothing is stored per-star across 100k+ galaxy nodes: calling `generate_star_system`
//! lazily reconstructs the host star and 4..=10 orbiting bodies in <0.5 ms from
//! `hash(map_seed, planet_id)` while anchoring the primary world to the live simulation
//! state (`hab`, terraforming progress, minerals, population, Stargate, and ECM Jammer).

use std::f32::consts::TAU;

use crate::types::{
    BodyType, MoonView, PlanetLiveStateView, StarKind, StarSystemView, StarView, SystemBodyView,
};

const ROMAN: [&str; 12] = [
    "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII",
];
const MOON_LETTERS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

/// Input snapshot for the anchored primary world from `Simulation`.
#[derive(Debug, Clone)]
pub struct PrimaryWorldInput {
    pub map_seed: u32,
    pub planet_id: usize, // 1-based display ID
    pub base_hab: f32,    // 0.0..=1.0
    pub terra_bonus: f32, // 0.0..~0.45 from owner's terraforming tech
    pub iron_crust: f32,
    pub bora_crust: f32,
    pub germ_crust: f32,
    pub owner: u8,
    pub owner_color: String,
    pub pop_norm: f32, // pop / max_pop in [0..1]
    pub has_starbase: bool,
    pub has_stargate: bool,
    pub has_jammer: bool,
}

/// Deterministic SplitMix64 PRNG for isolated on-demand generation.
struct SysRng {
    state: u64,
}

impl SysRng {
    fn from_seed(map_seed: u32, planet_id: usize) -> (Self, u32) {
        let mut z = ((map_seed as u64) << 32) ^ (planet_id as u64).wrapping_mul(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        let seed32 = (z as u32) ^ ((z >> 32) as u32);
        (Self { state: z | 1 }, seed32)
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

/// Derive physically consistent `(grav_g, temp_k, rad_mr)` from `base_hab`, `terra_bonus`, and RNG.
fn derive_primary_env(base_hab: f32, terra_bonus: f32, rng: &mut SysRng) -> (f32, f32, f32) {
    let h = base_hab.clamp(0.05, 1.0);
    let hostility = 1.0 - h; // 0.0 = Earth-like, 0.95 = extreme

    // Pick which environmental axis dominates hostility deterministically
    let mode = rng.next_f32();
    let sign = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };

    // Base Gravity (0.25g .. 2.8g), centered at 1.0g
    let grav_delta = if mode < 0.28 {
        hostility * sign * rng.range(0.75, 1.55)
    } else {
        hostility * sign * rng.range(0.15, 0.55)
    };
    let mut grav_g = (1.0 + grav_delta).clamp(0.22, 2.85);

    // Base Temperature (85 K .. 780 K), centered at 288 K
    let temp_delta = if mode < 0.68 {
        if sign > 0.0 {
            hostility * rng.range(120.0, 440.0)
        } else {
            -hostility * rng.range(90.0, 195.0)
        }
    } else {
        hostility * sign * rng.range(25.0, 95.0)
    };
    let mut temp_k = (288.0 + temp_delta).clamp(75.0, 820.0);

    // Base Radiation (5 mR .. 98 mR), centered at 32 mR
    let rad_delta = if mode >= 0.62 {
        hostility * rng.range(25.0, 66.0)
    } else {
        hostility * rng.range(5.0, 34.0)
    };
    let mut rad_mr = (32.0 + rad_delta).clamp(4.0, 98.0);

    // Live terraforming shifts temperature, gravity & radiation toward temperate Earth-like norms
    let t = terra_bonus.clamp(0.0, 0.75);
    if t > 0.0 {
        let blend = (t * 1.45).min(0.85);
        temp_k = temp_k + (288.0 - temp_k) * blend;
        grav_g = grav_g + (1.0 - grav_g) * (blend * 0.55);
        rad_mr = rad_mr + (28.0 - rad_mr) * (blend * 0.65);
    }

    (grav_g, temp_k, rad_mr)
}

/// Generate a host star consistent with the primary world's radiation and temperature,
/// including rare exotic stars (Brown Dwarf, White Dwarf, Pulsar, Black Hole).
fn generate_star(temp_k: f32, rad_mr: f32, rng: &mut SysRng) -> StarView {
    let seed = rng.next_u32();
    let exotic_roll = rng.next_f32();

    // Rare Black Hole (~2% on extreme radiation > 82 mR, or ~0.6% very rare overall)
    if (rad_mr > 82.0 && exotic_roll < 0.04) || exotic_roll < 0.006 {
        let mass = rng.range(4.8, 12.5);
        return StarView {
            star_kind: StarKind::BlackHole,
            spectral_class: "BH".to_string(),
            temp_k: rng.range(18000.0, 42000.0), // accretion disk inner temp
            mass_solar: mass,
            radius_solar: mass * 0.0000042,
            luminosity_solar: rng.range(0.35, 2.4),
            color_rgb: [1.0, 0.72, 0.36],
            seed,
        };
    }

    // Rare Pulsar / Neutron Star (~5% on extreme radiation > 76 mR, or ~1.2% overall)
    if (rad_mr > 76.0 && exotic_roll < 0.09) || exotic_roll < 0.018 {
        return StarView {
            star_kind: StarKind::Pulsar,
            spectral_class: format!("PSR-{}", (rng.next_u32() % 900) + 100),
            temp_k: rng.range(24000.0, 58000.0),
            mass_solar: rng.range(1.38, 2.15),
            radius_solar: 0.000018,
            luminosity_solar: rng.range(0.25, 1.4),
            color_rgb: [0.54, 0.86, 1.0],
            seed,
        };
    }

    // White Dwarf (~18% on high-rad / cooler worlds, or ~3.5% overall)
    if (rad_mr > 56.0 && temp_k < 360.0 && exotic_roll < 0.24) || exotic_roll < 0.052 {
        return StarView {
            star_kind: StarKind::WhiteDwarf,
            spectral_class: format!("DA{}", (rng.next_u32() % 6) + 2),
            temp_k: rng.range(10500.0, 24000.0),
            mass_solar: rng.range(0.54, 1.18),
            radius_solar: rng.range(0.009, 0.016),
            luminosity_solar: rng.range(0.04, 0.28),
            color_rgb: [0.84, 0.93, 1.0],
            seed,
        };
    }

    // Brown Dwarf (L / T / Y classes) for very cold systems
    if temp_k < 175.0 && exotic_roll < 0.34 {
        let sub = rng.next_f32();
        let (cls, bd_temp) = if sub < 0.45 {
            (format!("L{}", (rng.next_u32() % 8) + 1), rng.range(1350.0, 2100.0))
        } else if sub < 0.82 {
            (format!("T{}", (rng.next_u32() % 8) + 1), rng.range(720.0, 1300.0))
        } else {
            (format!("Y{}", (rng.next_u32() % 3) + 1), rng.range(420.0, 680.0))
        };
        return StarView {
            star_kind: StarKind::BrownDwarf,
            spectral_class: cls,
            temp_k: bd_temp,
            mass_solar: rng.range(0.028, 0.074),
            radius_solar: rng.range(0.085, 0.115),
            luminosity_solar: rng.range(0.015, 0.045),
            color_rgb: [0.92, 0.32, 0.44],
            seed,
        };
    }

    let (spectral_class, star_temp, mass, radius, color_rgb) = if rad_mr > 72.0 && temp_k > 390.0 {
        if rng.next_f32() < 0.65 {
            (
                format!("A{}V", (rng.next_u32() % 5) + 1),
                rng.range(7600.0, 9200.0),
                rng.range(1.65, 2.15),
                rng.range(1.55, 2.0),
                [0.76, 0.86, 1.0],
            )
        } else {
            (
                format!("M{}Ve", (rng.next_u32() % 4) + 1),
                rng.range(3100.0, 3650.0),
                rng.range(0.28, 0.48),
                rng.range(0.32, 0.52),
                [1.0, 0.44, 0.28],
            )
        }
    } else if rad_mr > 54.0 || temp_k > 350.0 {
        (
            format!("F{}V", (rng.next_u32() % 7) + 2),
            rng.range(6100.0, 7250.0),
            rng.range(1.15, 1.55),
            rng.range(1.18, 1.48),
            [0.95, 0.96, 1.0],
        )
    } else if temp_k < 185.0 {
        if rng.next_f32() < 0.60 {
            (
                format!("M{}V", (rng.next_u32() % 6) + 1),
                rng.range(2850.0, 3600.0),
                rng.range(0.22, 0.52),
                rng.range(0.26, 0.56),
                [1.0, 0.52, 0.32],
            )
        } else {
            (
                format!("K{}V", (rng.next_u32() % 7) + 2),
                rng.range(3950.0, 5050.0),
                rng.range(0.58, 0.84),
                rng.range(0.65, 0.86),
                [1.0, 0.78, 0.52],
            )
        }
    } else if rng.next_f32() < 0.58 {
        (
            format!("G{}V", (rng.next_u32() % 8) + 1),
            rng.range(5350.0, 6020.0),
            rng.range(0.88, 1.12),
            rng.range(0.90, 1.14),
            [1.0, 0.95, 0.80],
        )
    } else {
        (
            format!("K{}V", (rng.next_u32() % 6) + 1),
            rng.range(4300.0, 5200.0),
            rng.range(0.68, 0.88),
            rng.range(0.72, 0.89),
            [1.0, 0.82, 0.56],
        )
    };

    // Main-sequence mass-luminosity approximation L ~ M^3.5
    let luminosity_solar = mass.powf(3.5).clamp(0.015, 22.0);
    StarView {
        star_kind: StarKind::MainSequence,
        spectral_class,
        temp_k: star_temp,
        mass_solar: mass,
        radius_solar: radius,
        luminosity_solar,
        color_rgb,
        seed,
    }
}

/// Optional binary companion star (~30% of systems) creating dual-tinted illumination on planets.
fn generate_companion_star(primary: &StarView, rng: &mut SysRng) -> Option<StarView> {
    if matches!(primary.star_kind, StarKind::BlackHole | StarKind::Pulsar) {
        return None;
    }
    if rng.next_f32() >= 0.30 {
        return None;
    }
    let seed = rng.next_u32();
    let roll = rng.next_f32();
    if roll < 0.48 {
        let mass = rng.range(0.18, 0.48);
        Some(StarView {
            star_kind: StarKind::MainSequence,
            spectral_class: format!("M{}V", (rng.next_u32() % 7) + 1),
            temp_k: rng.range(2850.0, 3650.0),
            mass_solar: mass,
            radius_solar: rng.range(0.22, 0.50),
            luminosity_solar: mass.powf(3.5).clamp(0.01, 0.12),
            color_rgb: [1.0, 0.48, 0.28],
            seed,
        })
    } else if roll < 0.76 {
        let mass = rng.range(0.55, 0.82);
        Some(StarView {
            star_kind: StarKind::MainSequence,
            spectral_class: format!("K{}V", (rng.next_u32() % 7) + 2),
            temp_k: rng.range(4000.0, 4950.0),
            mass_solar: mass,
            radius_solar: rng.range(0.62, 0.84),
            luminosity_solar: mass.powf(3.5).clamp(0.08, 0.55),
            color_rgb: [1.0, 0.78, 0.48],
            seed,
        })
    } else if roll < 0.90 {
        Some(StarView {
            star_kind: StarKind::WhiteDwarf,
            spectral_class: format!("DA{}", (rng.next_u32() % 5) + 2),
            temp_k: rng.range(12000.0, 22000.0),
            mass_solar: rng.range(0.55, 0.95),
            radius_solar: 0.012,
            luminosity_solar: rng.range(0.04, 0.22),
            color_rgb: [0.76, 0.90, 1.0],
            seed,
        })
    } else {
        Some(StarView {
            star_kind: StarKind::BrownDwarf,
            spectral_class: format!("L{}", (rng.next_u32() % 7) + 2),
            temp_k: rng.range(1150.0, 1950.0),
            mass_solar: rng.range(0.035, 0.072),
            radius_solar: 0.095,
            luminosity_solar: 0.018,
            color_rgb: [0.92, 0.30, 0.44],
            seed,
        })
    }
}

fn classify_and_style_body(
    orbit_au: f32,
    frost_line_au: f32,
    temp_k: f32,
    radius_earth: f32,
    is_gas: bool,
    hab_hint: Option<f32>,
    rng: &mut SysRng,
) -> (BodyType, f32, f32, f32, [f32; 3], f32, bool, f32, f32, [f32; 3], u8) {
    if is_gas {
        let is_ice_giant = orbit_au > frost_line_au * 2.1 || radius_earth < 6.0;
        let body_type = if is_ice_giant {
            BodyType::IceGiant
        } else {
            BodyType::GasGiant
        };
        let atmo_color = if is_ice_giant {
            [
                rng.range(0.32, 0.55),
                rng.range(0.66, 0.86),
                rng.range(0.88, 1.0),
            ]
        } else {
            [
                rng.range(0.76, 0.96),
                rng.range(0.62, 0.82),
                rng.range(0.42, 0.66),
            ]
        };
        let has_rings = rng.next_f32() < if is_ice_giant { 0.42 } else { 0.55 };
        let ring_inner = rng.range(1.28, 1.52);
        let ring_outer = ring_inner + rng.range(0.55, 1.05);
        let ring_color = if is_ice_giant {
            [0.62, 0.78, 0.92]
        } else {
            [0.86, 0.78, 0.64]
        };
        let moons = (2 + (rng.next_u32() % if is_ice_giant { 5 } else { 9 })) as u8;
        return (
            body_type,
            0.0,
            0.0,
            1.0,
            atmo_color,
            rng.range(0.65, 0.95),
            has_rings,
            ring_inner,
            ring_outer,
            ring_color,
            moons,
        );
    }

    // Rocky / terrestrial classification
    let eff_hab = hab_hint.unwrap_or_else(|| {
        let t_ok = 1.0 - ((temp_k - 288.0).abs() / 160.0).min(1.0);
        let r_ok = 1.0 - ((radius_earth - 1.0).abs() / 1.2).min(1.0);
        (t_ok * 0.7 + r_ok * 0.3).clamp(0.0, 1.0)
    });

    let (body_type, water_pct, ice_pct, atmo_density, atmo_color, cloud_pct) = if temp_k >= 610.0 {
        (
            BodyType::Volcanic,
            0.0,
            0.0,
            rng.range(0.18, 0.65),
            [0.98, 0.48, 0.22],
            rng.range(0.08, 0.35),
        )
    } else if temp_k <= 205.0 {
        let is_barren_rock = radius_earth < 0.52 && rng.next_f32() < 0.45;
        if is_barren_rock {
            (
                BodyType::Rock,
                0.0,
                rng.range(0.05, 0.25),
                rng.range(0.0, 0.06),
                [0.62, 0.68, 0.76],
                0.0,
            )
        } else {
            (
                BodyType::Ice,
                0.0,
                rng.range(0.65, 0.96),
                rng.range(0.08, 0.55),
                [0.64, 0.84, 0.98],
                rng.range(0.05, 0.32),
            )
        }
    } else if radius_earth < 0.48 && eff_hab < 0.45 {
        (
            BodyType::Rock,
            0.0,
            0.0,
            rng.range(0.0, 0.07),
            [0.72, 0.70, 0.66],
            0.0,
        )
    } else if temp_k >= 355.0 {
        (
            BodyType::Desert,
            rng.range(0.0, 0.14),
            0.0,
            rng.range(0.22, 0.78),
            [0.94, 0.72, 0.46],
            rng.range(0.04, 0.26),
        )
    } else {
        // Temperate zone (205 K .. 355 K)
        let water_base = (eff_hab * 0.72 + rng.range(-0.14, 0.22)).clamp(0.08, 0.92);
        let ice_base = if temp_k < 265.0 {
            ((265.0 - temp_k) / 60.0).clamp(0.12, 0.78)
        } else {
            ((295.0 - temp_k) / 140.0).clamp(0.04, 0.22)
        };
        if water_base > 0.74 {
            (
                BodyType::Oceanic,
                water_base,
                ice_base,
                rng.range(0.62, 1.0),
                [0.34, 0.68, 1.0],
                rng.range(0.32, 0.66),
            )
        } else if water_base < 0.22 {
            (
                BodyType::Desert,
                water_base,
                ice_base * 0.4,
                rng.range(0.28, 0.68),
                [0.88, 0.74, 0.54],
                rng.range(0.08, 0.28),
            )
        } else {
            (
                BodyType::Terrestrial,
                water_base,
                ice_base,
                rng.range(0.55, 0.98),
                [0.40, 0.72, 1.0],
                rng.range(0.24, 0.56),
            )
        }
    };

    let has_rings = radius_earth > 1.25 && rng.next_f32() < 0.10;
    let ring_inner = 1.38;
    let ring_outer = 1.92;
    let ring_color = [0.78, 0.75, 0.70];
    let moons = if radius_earth > 0.75 && rng.next_f32() < 0.58 {
        (1 + (rng.next_u32() % 2)) as u8
    } else {
        0
    };

    (
        body_type,
        water_pct,
        ice_pct,
        atmo_density,
        atmo_color,
        cloud_pct,
        has_rings,
        ring_inner,
        ring_outer,
        ring_color,
        moons,
    )
}

fn generate_moons(
    parent_name: &str,
    parent_type: BodyType,
    parent_temp_k: f32,
    parent_has_rings: bool,
    ring_outer: f32,
    orbit_au: f32,
    frost_line_au: f32,
    count: u8,
    rng: &mut SysRng,
) -> Vec<MoonView> {
    let capped = (count as usize).min(6);
    if capped == 0 {
        return Vec::new();
    }
    let is_giant = matches!(parent_type, BodyType::GasGiant | BodyType::IceGiant);
    let mut moons = Vec::with_capacity(capped);
    let mut cur_r = if parent_has_rings {
        (ring_outer + 0.32).max(1.85)
    } else {
        rng.range(1.65, 2.05)
    };

    for m in 0..capped {
        if m > 0 {
            cur_r += rng.range(0.36, 0.58);
        }
        let letter = MOON_LETTERS.get(m).copied().unwrap_or("x");
        let name = format!("{parent_name}-{letter}");
        let seed = rng.next_u32();
        let orbit_angle_rad = rng.range(0.0, TAU);
        // Keplerian-like angular speed: inner moons orbit faster
        let orbit_speed = (0.72 / cur_r.powf(1.25)).clamp(0.12, 0.55);
        let radius_earth = if is_giant {
            rng.range(0.16, 0.42)
        } else {
            rng.range(0.12, 0.29)
        };

        // Tidal heating on innermost giant moon (Io-like), icy shells beyond frost line,
        // Titan-like hazy moon occasionally around gas giants, or cratered rocky moons
        let (body_type, temp_k, water_pct, ice_pct, atmo_density, atmo_color_rgb, cloud_pct) =
            if is_giant && m == 0 && rng.next_f32() < 0.45 {
                (
                    BodyType::Volcanic,
                    rng.range(640.0, 790.0),
                    0.0,
                    0.0,
                    rng.range(0.05, 0.22),
                    [0.98, 0.54, 0.24],
                    rng.range(0.0, 0.14),
                )
            } else if orbit_au >= frost_line_au * 0.9 || parent_temp_k < 210.0 {
                if is_giant && radius_earth > 0.30 && rng.next_f32() < 0.28 {
                    // Titan-like hazy icy/organic moon
                    (
                        BodyType::Desert,
                        parent_temp_k.clamp(85.0, 145.0),
                        0.08,
                        0.15,
                        rng.range(0.55, 0.85),
                        [0.92, 0.68, 0.34],
                        rng.range(0.35, 0.65),
                    )
                } else {
                    (
                        BodyType::Ice,
                        parent_temp_k.clamp(55.0, 185.0),
                        0.0,
                        rng.range(0.68, 0.96),
                        rng.range(0.0, 0.08),
                        [0.68, 0.84, 0.98],
                        0.0,
                    )
                }
            } else {
                (
                    BodyType::Rock,
                    parent_temp_k.clamp(160.0, 520.0),
                    0.0,
                    if parent_temp_k < 250.0 {
                        rng.range(0.05, 0.25)
                    } else {
                        0.0
                    },
                    rng.range(0.0, 0.05),
                    [0.72, 0.72, 0.74],
                    0.0,
                )
            };

        moons.push(MoonView {
            name,
            orbit_radius_planet: cur_r,
            orbit_angle_rad,
            orbit_speed,
            radius_earth,
            body_type,
            temp_k,
            water_pct,
            ice_pct,
            atmo_density,
            atmo_color_rgb,
            cloud_pct,
            seed,
        });
    }
    moons
}

/// Generate a complete, physically consistent `StarSystemView` anchored to `input`.
pub fn generate_star_system(input: &PrimaryWorldInput) -> StarSystemView {
    let (mut rng, system_seed) = SysRng::from_seed(input.map_seed, input.planet_id);

    // 1. Derive primary world's astrophysical environment & matching host star (+ optional binary companion)
    let (grav_g, primary_temp_k, rad_mr) =
        derive_primary_env(input.base_hab, input.terra_bonus, &mut rng);
    let star = generate_star(primary_temp_k, rad_mr, &mut rng);
    let companion_star = generate_companion_star(&star, &mut rng);

    // Equilibrium orbital radius for the primary world: T_eq ~ 278 * L^0.25 / sqrt(a)
    let lum_quarter = star.luminosity_solar.powf(0.25);
    let frost_line_au = (2.7 * star.luminosity_solar.sqrt()).clamp(0.45, 14.0);
    let primary_orbit_au = ((278.0 * lum_quarter / primary_temp_k.max(70.0)).powi(2))
        .clamp(0.12 * lum_quarter, 18.0 * lum_quarter);

    // 2. Dole/Fogg geometric Bode-like accretion spacing (4..=10 bodies total)
    let total_bodies = (4 + (rng.next_u32() % 7)) as usize; // 4..=10
    let inner_count = if primary_orbit_au < 0.35 * lum_quarter {
        0
    } else {
        (rng.next_u32() as usize % (total_bodies.min(4))).min(total_bodies - 1)
    };
    let outer_count = total_bodies - 1 - inner_count;

    let mut orbits: Vec<f32> = Vec::with_capacity(total_bodies);

    // Build inner orbits stepping inward from primary_orbit_au
    let mut inner_orbits: Vec<f32> = Vec::with_capacity(inner_count);
    let mut cur_in = primary_orbit_au;
    for _ in 0..inner_count {
        cur_in /= rng.range(1.48, 1.92);
        if cur_in < 0.045 {
            break;
        }
        inner_orbits.push(cur_in);
    }
    inner_orbits.reverse();
    let mut primary_index = inner_orbits.len();
    orbits.extend_from_slice(&inner_orbits);
    orbits.push(primary_orbit_au);

    // Build outer orbits stepping outward from primary_orbit_au
    let mut cur_out = primary_orbit_au;
    for _ in 0..outer_count {
        cur_out *= rng.range(1.52, 2.08);
        orbits.push(cur_out);
    }

    // Ensure at least 4 bodies even if primary was very close to star
    while orbits.len() < 4 {
        cur_out *= rng.range(1.55, 2.0);
        orbits.push(cur_out);
    }

    // Compute normalized mineral tint [Fe, Bo, Ge] for the primary world
    let m_sum = (input.iron_crust + input.bora_crust + input.germ_crust).max(1.0);
    let mineral_tint = [
        (input.iron_crust / m_sum).clamp(0.0, 1.0),
        (input.bora_crust / m_sum).clamp(0.0, 1.0),
        (input.germ_crust / m_sum).clamp(0.0, 1.0),
    ];

    // 3. Instantiate all bodies along the accreted orbits
    let mut bodies = Vec::with_capacity(orbits.len() + 3);
    for (idx, &orbit_au) in orbits.iter().enumerate() {
        let body_seed = rng.next_u32();
        let angle = rng.range(0.0, TAU);
        let suffix = ROMAN.get(idx).copied().unwrap_or("X");
        let name = format!("System #{} {}", input.planet_id, suffix);

        if idx == primary_index {
            // Anchored Primary World: directly driven by SI-STARS simulation state
            let eff_hab = (input.base_hab + input.terra_bonus).clamp(0.05, 1.0);
            let radius_earth = grav_g.powf(0.45).clamp(0.55, 1.75);
            let (
                body_type,
                water_pct,
                ice_pct,
                atmo_density,
                atmo_color_rgb,
                cloud_pct,
                has_rings,
                ring_inner,
                ring_outer,
                ring_color_rgb,
                moons_count,
            ) = classify_and_style_body(
                orbit_au,
                frost_line_au,
                primary_temp_k,
                radius_earth,
                false,
                Some(eff_hab),
                &mut rng,
            );

            let moons = generate_moons(
                &name,
                body_type,
                primary_temp_k,
                has_rings,
                ring_outer,
                orbit_au,
                frost_line_au,
                moons_count,
                &mut rng,
            );
            let actual_moons_count = moons.len() as u8;

            bodies.push(SystemBodyView {
                name,
                orbit_au,
                orbit_angle_rad: angle,
                radius_earth,
                body_type,
                temp_k: primary_temp_k,
                water_pct,
                ice_pct,
                atmo_density,
                atmo_color_rgb,
                cloud_pct,
                has_rings,
                ring_inner,
                ring_outer,
                ring_color_rgb,
                moons_count: actual_moons_count,
                moons,
                seed: body_seed,
                live_state: Some(PlanetLiveStateView {
                    owner: input.owner,
                    owner_color: input.owner_color.clone(),
                    pop_norm: input.pop_norm.clamp(0.0, 1.0),
                    has_starbase: input.has_starbase,
                    has_stargate: input.has_stargate,
                    has_jammer: input.has_jammer,
                    mineral_tint,
                    grav_g,
                    rad_mr,
                    terra_pct: (input.terra_bonus * 100.0).clamp(0.0, 100.0),
                }),
            });
        } else {
            // Companion accreted planet (or Dwarf / Plutoid on outer/tiny orbits)
            let eq_temp = (278.0 * lum_quarter / orbit_au.sqrt()).clamp(38.0, 1450.0);
            let beyond_frost = orbit_au >= frost_line_au * 0.92;
            let is_outermost = idx + 1 == orbits.len();
            let is_dwarf =
                (is_outermost && beyond_frost && rng.next_f32() < 0.42)
                    || (!beyond_frost && orbit_au > frost_line_au * 0.62 && rng.next_f32() < 0.16);

            if is_dwarf {
                let radius_earth = rng.range(0.16, 0.34);
                let is_plutoid = eq_temp < 140.0;
                let dwarf_ice = if is_plutoid {
                    rng.range(0.45, 0.82)
                } else {
                    rng.range(0.08, 0.28)
                };
                let moons_n = if is_plutoid && rng.next_f32() < 0.45 { 1 } else { 0 };
                let moons = generate_moons(
                    &name,
                    BodyType::Dwarf,
                    eq_temp,
                    false,
                    0.0,
                    orbit_au,
                    frost_line_au,
                    moons_n,
                    &mut rng,
                );
                let actual_moons_count = moons.len() as u8;
                bodies.push(SystemBodyView {
                    name,
                    orbit_au,
                    orbit_angle_rad: angle,
                    radius_earth,
                    body_type: BodyType::Dwarf,
                    temp_k: eq_temp,
                    water_pct: 0.0,
                    ice_pct: dwarf_ice,
                    atmo_density: rng.range(0.0, 0.04),
                    atmo_color_rgb: if is_plutoid {
                        [0.72, 0.80, 0.92]
                    } else {
                        [0.68, 0.64, 0.60]
                    },
                    cloud_pct: 0.0,
                    has_rings: false,
                    ring_inner: 0.0,
                    ring_outer: 0.0,
                    ring_color_rgb: [0.7, 0.7, 0.7],
                    moons_count: actual_moons_count,
                    moons,
                    seed: body_seed,
                    live_state: None,
                });
                continue;
            }

            let is_gas = beyond_frost && rng.next_f32() < 0.72;
            let radius_earth = if is_gas {
                if orbit_au > frost_line_au * 2.2 {
                    rng.range(3.6, 5.8)
                } else {
                    rng.range(6.8, 12.2)
                }
            } else {
                rng.range(0.35, 1.65)
            };

            let (
                body_type,
                water_pct,
                ice_pct,
                atmo_density,
                atmo_color_rgb,
                cloud_pct,
                has_rings,
                ring_inner,
                ring_outer,
                ring_color_rgb,
                moons_count,
            ) = classify_and_style_body(
                orbit_au,
                frost_line_au,
                eq_temp,
                radius_earth,
                is_gas,
                None,
                &mut rng,
            );

            let moons = generate_moons(
                &name,
                body_type,
                eq_temp,
                has_rings,
                ring_outer,
                orbit_au,
                frost_line_au,
                moons_count,
                &mut rng,
            );
            let actual_moons_count = moons.len() as u8;

            bodies.push(SystemBodyView {
                name,
                orbit_au,
                orbit_angle_rad: angle,
                radius_earth,
                body_type,
                temp_k: eq_temp,
                water_pct,
                ice_pct,
                atmo_density,
                atmo_color_rgb,
                cloud_pct,
                has_rings,
                ring_inner,
                ring_outer,
                ring_color_rgb,
                moons_count: actual_moons_count,
                moons,
                seed: body_seed,
                live_state: None,
            });
        }
    }

    // 4. Optional intra-system astrophysical features: Main Asteroid Belt, Kuiper Belt & Comet
    // 4a. Main Asteroid Belt (near frost line / rocky-to-giant transition)
    if rng.next_f32() < 0.68 {
        let mut insert_pos = None;
        for i in 0..bodies.len().saturating_sub(1) {
            let a = bodies[i].orbit_au;
            let b = bodies[i + 1].orbit_au;
            let is_rocky_a = !matches!(bodies[i].body_type, BodyType::GasGiant | BodyType::IceGiant);
            let is_giant_b = matches!(bodies[i + 1].body_type, BodyType::GasGiant | BodyType::IceGiant);
            if (is_rocky_a && is_giant_b) || (a < frost_line_au && b >= frost_line_au) {
                insert_pos = Some((i + 1, (a + b) * 0.5));
                break;
            }
        }
        if let Some((pos, belt_au)) = insert_pos {
            let eq_temp = (278.0 * lum_quarter / belt_au.sqrt()).clamp(60.0, 420.0);
            bodies.insert(
                pos,
                SystemBodyView {
                    name: "Main Belt".to_string(),
                    orbit_au: belt_au,
                    orbit_angle_rad: rng.range(0.0, TAU),
                    radius_earth: 0.0,
                    body_type: BodyType::AsteroidBelt,
                    temp_k: eq_temp,
                    water_pct: 0.0,
                    ice_pct: 0.12,
                    atmo_density: 0.0,
                    atmo_color_rgb: [0.72, 0.66, 0.58],
                    cloud_pct: 0.0,
                    has_rings: false,
                    ring_inner: 0.52,
                    ring_outer: 0.92,
                    ring_color_rgb: [0.76, 0.68, 0.58],
                    moons_count: 0,
                    moons: Vec::new(),
                    seed: rng.next_u32(),
                    live_state: None,
                },
            );
            if pos <= primary_index {
                primary_index += 1;
            }
        }
    }

    // 4b. Kuiper Belt (outer icy rim beyond the outermost planet)
    if bodies.len() >= 5 && rng.next_f32() < 0.72 {
        let outer_au = bodies.last().map(|b| b.orbit_au).unwrap_or(5.0) * rng.range(1.38, 1.65);
        let eq_temp = (278.0 * lum_quarter / outer_au.sqrt()).clamp(28.0, 95.0);
        bodies.push(SystemBodyView {
            name: "Kuiper Belt".to_string(),
            orbit_au: outer_au,
            orbit_angle_rad: rng.range(0.0, TAU),
            radius_earth: 0.0,
            body_type: BodyType::KuiperBelt,
            temp_k: eq_temp,
            water_pct: 0.0,
            ice_pct: 0.92,
            atmo_density: 0.0,
            atmo_color_rgb: [0.64, 0.84, 0.98],
            cloud_pct: 0.0,
            has_rings: false,
            ring_inner: 0.48,
            ring_outer: 0.95,
            ring_color_rgb: [0.68, 0.86, 1.0],
            moons_count: 0,
            moons: Vec::new(),
            seed: rng.next_u32(),
            live_state: None,
        });
    }

    // 4c. Rare eccentric Comet (~26% of systems)
    if rng.next_f32() < 0.26 {
        let comet_au = (bodies.last().map(|b| b.orbit_au).unwrap_or(6.0) * rng.range(1.18, 1.45))
            .max(1.2);
        let eq_temp = (278.0 * lum_quarter / comet_au.sqrt()).clamp(45.0, 260.0);
        let code = format!("C/{}-{}", 2400 + (system_seed % 95), (system_seed % 26) + 1);
        bodies.push(SystemBodyView {
            name: format!("Comet {code}"),
            orbit_au: comet_au,
            orbit_angle_rad: rng.range(0.0, TAU),
            radius_earth: rng.range(0.01, 0.03),
            body_type: BodyType::Comet,
            temp_k: eq_temp,
            water_pct: 0.0,
            ice_pct: 0.85,
            atmo_density: 0.22,
            atmo_color_rgb: [0.48, 0.82, 1.0],
            cloud_pct: 0.0,
            has_rings: false,
            ring_inner: 0.0,
            ring_outer: 0.0,
            ring_color_rgb: [0.55, 0.82, 1.0],
            moons_count: 0,
            moons: Vec::new(),
            seed: rng.next_u32(),
            live_state: None,
        });
    }

    StarSystemView {
        planet_id: input.planet_id,
        system_seed,
        star,
        companion_star,
        primary_index,
        bodies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(planet_id: usize, hab: f32, terra: f32, pop_norm: f32) -> PrimaryWorldInput {
        PrimaryWorldInput {
            map_seed: 123456,
            planet_id,
            base_hab: hab,
            terra_bonus: terra,
            iron_crust: 800.0,
            bora_crust: 600.0,
            germ_crust: 900.0,
            owner: 1,
            owner_color: "#38bdf8".to_string(),
            pop_norm,
            has_starbase: true,
            has_stargate: true,
            has_jammer: true,
        }
    }

    #[test]
    fn deterministic_and_valid_orbits() {
        let inp = sample_input(42, 0.85, 0.0, 0.5);
        let a = generate_star_system(&inp);
        let b = generate_star_system(&inp);

        assert_eq!(a.system_seed, b.system_seed);
        assert_eq!(a.primary_index, b.primary_index);
        assert_eq!(a.bodies.len(), b.bodies.len());
        assert!((4..=13).contains(&a.bodies.len()));
        assert!(a.primary_index < a.bodies.len());

        // Strictly increasing orbits
        for w in a.bodies.windows(2) {
            assert!(w[1].orbit_au > w[0].orbit_au);
        }

        // Only primary body carries live_state, and all moons have valid strictly increasing orbits
        for (i, body) in a.bodies.iter().enumerate() {
            assert_eq!(body.live_state.is_some(), i == a.primary_index);
            assert_eq!(body.moons.len(), body.moons_count as usize);
            for m in &body.moons {
                assert!(m.orbit_radius_planet > 1.4);
            }
            for w in body.moons.windows(2) {
                assert!(w[1].orbit_radius_planet > w[0].orbit_radius_planet);
            }
        }
    }

    #[test]
    fn live_state_and_terraforming_affect_primary_world() {
        let raw = generate_star_system(&sample_input(77, 0.25, 0.0, 0.1));
        let terra = generate_star_system(&sample_input(77, 0.25, 0.40, 0.85));

        let p_raw = &raw.bodies[raw.primary_index];
        let p_terra = &terra.bodies[terra.primary_index];

        // Terraforming moderates temperature closer to 288 K
        assert!(
            (p_terra.temp_k - 288.0).abs() < (p_raw.temp_k - 288.0).abs(),
            "expected terraforming to moderate temp: raw={}K, terra={}K",
            p_raw.temp_k,
            p_terra.temp_k
        );

        let ls = p_terra.live_state.as_ref().expect("primary live state");
        assert!((ls.pop_norm - 0.85).abs() < 1e-4);
        assert!(ls.has_starbase);
        assert!(ls.has_stargate);
        assert!(ls.has_jammer);
    }

    #[test]
    fn generates_belts_dwarfs_and_comets_across_systems() {
        let mut saw_belt = false;
        let mut saw_kuiper = false;
        let mut saw_dwarf = false;
        let mut saw_comet = false;
        for id in 1..=60 {
            let sys = generate_star_system(&sample_input(id, 0.65, 0.0, 0.2));
            assert!(sys.bodies[sys.primary_index].live_state.is_some());
            for w in sys.bodies.windows(2) {
                assert!(w[1].orbit_au > w[0].orbit_au);
            }
            for b in &sys.bodies {
                match b.body_type {
                    BodyType::AsteroidBelt => saw_belt = true,
                    BodyType::KuiperBelt => saw_kuiper = true,
                    BodyType::Dwarf => saw_dwarf = true,
                    BodyType::Comet => saw_comet = true,
                    _ => {}
                }
            }
        }
        assert!(saw_belt, "expected Main Asteroid Belt in at least one system");
        assert!(saw_kuiper, "expected Kuiper Belt in at least one system");
        assert!(saw_dwarf, "expected Dwarf planet in at least one system");
        assert!(saw_comet, "expected Comet in at least one system");
    }

    #[test]
    fn generates_binary_and_exotic_stars() {
        let mut saw_binary = false;
        let mut saw_brown = false;
        let mut saw_white = false;
        let mut saw_pulsar = false;
        let mut saw_bh = false;
        for id in 1..=350 {
            let hab = if id % 3 == 0 { 0.12 } else { 0.55 };
            let sys = generate_star_system(&sample_input(id, hab, 0.0, 0.1));
            if sys.companion_star.is_some() {
                saw_binary = true;
            }
            match sys.star.star_kind {
                StarKind::BrownDwarf => saw_brown = true,
                StarKind::WhiteDwarf => saw_white = true,
                StarKind::Pulsar => saw_pulsar = true,
                StarKind::BlackHole => saw_bh = true,
                StarKind::MainSequence => {}
            }
        }
        assert!(saw_binary, "expected Binary star system");
        assert!(saw_brown, "expected Brown Dwarf");
        assert!(saw_white, "expected White Dwarf");
        assert!(saw_pulsar, "expected Pulsar");
        assert!(saw_bh, "expected Black Hole");
    }
}
