// Vendored from jcode (crates/operant-tui-anim/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; internal
// crate self-references re-rooted to crate::tui::operant_anim. The four
// vendored samplers (donut, gyroscope, black_hole, orbit_rings) and their
// angle-table machinery were deleted in the W4 animation swap; `sample_signal`
// below is ORIGINAL operant code, not a jcode port. `shape_char_3x3` and
// `hsv_to_rgb` remain vendored verbatim.

//! Pure, dependency-free math kernels for the TUI idle animations.
//!
//! This module intentionally contains only hot, self-contained numeric code
//! (the 2D radar-pulse sampler, the 3x3 subpixel glyph chooser and the HSV->RGB
//! conversion). It is split out of `operant-tui` so it can be pinned to
//! `opt-level = 3` via per-package profile overrides in the workspace
//! `Cargo.toml`, keeping the per-subpixel trig fully optimized even in
//! `dev`/`selfdev` debug builds.
//!
//! `sample_signal` is deterministic by construction: every input derives from
//! `elapsed` alone (no clock, no randomness), and it spends exactly two
//! transcendentals per subpixel (`sqrt` for the radius, `atan2` for the beam
//! angle) with rational falloffs everywhere else — no lookup tables are needed
//! at the ~2.7k-subpixel scale of a typical idle area, unlike the deleted
//! samplers' ~142k `cos`/`sin` per frame.

#![cfg_attr(test, allow(dead_code))]

/// Sweep speed of the radar beam head, radians per second.
const BEAM_SPEED: f32 = 1.6;
/// Afterglow: falloff of intensity behind the beam head, in radians.
/// Rational falloff (no exp) keeps this at 2 transcendentals per subpixel.
const BEAM_TRAIL: f32 = 0.55;
/// Expanding-ring period in seconds (three rings staggered by 1/3 phase).
const RING_PERIOD: f32 = 3.2;
/// Half-width of a ring band in unit-disk space.
const RING_WIDTH: f32 = 0.09;
/// Subpixels dimmer than this are left unpainted (hit = false).
const PAINT_THRESHOLD: f32 = 0.035;

/// The idle animation: an original operant radar-pulse. A beam sweeps the unit
/// disk leaving a decaying afterglow, three staggered rings expand outward
/// from the center, and a bright signal core pulses at the origin. See the
/// module doc for the determinism and cost contract.

pub fn sample_signal(
    elapsed: f32,
    sw: usize,
    sh: usize,
    hit: &mut [bool],
    lum_map: &mut [f32],
    z_buf: &mut [f32],
) {
    // Unit-disk space: terminal cells are ~2x taller than wide, so each axis
    // is normalized by its own half-extent — the sweep reads circular even
    // though the subpixel grid is anisotropic.
    let nx = sw as f32 * 0.5;
    let ny = sh as f32 * 0.5;
    let head = elapsed * BEAM_SPEED;

    // Staggered ring phases, precomputed per frame.
    let mut ring_r = [0.0f32; 3];
    for (i, slot) in ring_r.iter_mut().enumerate() {
        let phase = (elapsed / RING_PERIOD + i as f32 / 3.0).rem_euclid(1.0);
        *slot = phase;
    }

    for py in 0..sh {
        let dy = (py as f32 - ny + 0.5) / ny;
        for px in 0..sw {
            let dx = (px as f32 - nx + 0.5) / nx;
            let idx = py * sw + px;

            // Transcendental 1 of 2: the radius in unit-disk space.
            let r = (dx * dx + dy * dy).sqrt();
            if r > 1.0 {
                continue; // outside the unit disk: leave the subpixel untouched
            }

            // Transcendental 2 of 2: the sweep angle.
            let angle = dy.atan2(dx);

            // Beam afterglow: brightest at the head, rational falloff behind.
            let behind = (head - angle).rem_euclid(std::f32::consts::TAU);
            let beam = 1.0 / (1.0 + behind * behind * BEAM_TRAIL * BEAM_TRAIL * 2.5);

            // Three staggered expanding rings; each is a rational band.
            let mut rings = 0.0f32;
            for &rr in &ring_r {
                let d = (r - rr) / RING_WIDTH;
                rings += 1.0 / (1.0 + d * d * 3.0);
            }
            rings *= 0.30;

            // Signal core at the center.
            let core = 1.0 / (1.0 + (r / 0.10) * (r / 0.10) * 3.0);

            // Distance falloff so the display fades at the rim.
            let edge = 1.0 - r * r;

            let intensity = (beam + rings + core * 0.9) * edge;
            if intensity > z_buf[idx] {
                z_buf[idx] = intensity;
                lum_map[idx] = intensity * 2.0 - 1.0;
                hit[idx] = intensity > PAINT_THRESHOLD;
            }
        }
    }
}


pub fn shape_char_3x3(pattern: u16, brightness: f32) -> char {
    if pattern == 0 {
        return ' ';
    }

    let top_l = pattern & 1 != 0;
    let top_c = pattern & 2 != 0;
    let top_r = pattern & 4 != 0;
    let mid_l = pattern & 8 != 0;
    let mid_c = pattern & 16 != 0;
    let mid_r = pattern & 32 != 0;
    let bot_l = pattern & 64 != 0;
    let bot_c = pattern & 128 != 0;
    let bot_r = pattern & 256 != 0;

    let count = pattern.count_ones();
    let top = (top_l as u8) + (top_c as u8) + (top_r as u8);
    let mid = (mid_l as u8) + (mid_c as u8) + (mid_r as u8);
    let bot = (bot_l as u8) + (bot_c as u8) + (bot_r as u8);
    let left = (top_l as u8) + (mid_l as u8) + (bot_l as u8);
    let center = (top_c as u8) + (mid_c as u8) + (bot_c as u8);
    let right = (top_r as u8) + (mid_r as u8) + (bot_r as u8);

    let bl = if brightness > 0.65 {
        2u8
    } else if brightness > 0.35 {
        1u8
    } else {
        0u8
    };

    if count >= 8 {
        return match bl {
            2 => '@',
            1 => '#',
            _ => '%',
        };
    }
    if count >= 7 {
        return match bl {
            2 => '#',
            1 => '%',
            _ => '*',
        };
    }

    if top_l && mid_c && bot_r && !top_r && !bot_l {
        return match bl {
            2 => '\\',
            1 => '\\',
            _ => '.',
        };
    }
    if top_r && mid_c && bot_l && !top_l && !bot_r {
        return match bl {
            2 => '/',
            1 => '/',
            _ => '.',
        };
    }

    if mid >= 2 && top <= 1 && bot <= 1 && mid > top && mid > bot {
        return match bl {
            2 => '=',
            1 => '-',
            _ => '~',
        };
    }
    if top >= 2 && mid <= 1 && bot == 0 {
        return match bl {
            2 => '=',
            1 => '-',
            _ => '~',
        };
    }
    if bot >= 2 && mid <= 1 && top == 0 {
        return match bl {
            2 => '=',
            1 => '_',
            _ => '.',
        };
    }

    if center >= 2 && left <= 1 && right <= 1 && center > left && center > right {
        return match bl {
            2 => '|',
            1 => '|',
            _ => ':',
        };
    }
    if left >= 2 && center <= 1 && right == 0 {
        return match bl {
            2 => '|',
            1 => '|',
            _ => ':',
        };
    }
    if right >= 2 && center <= 1 && left == 0 {
        return match bl {
            2 => '|',
            1 => '|',
            _ => ':',
        };
    }

    if top >= 2 && bot == 0 {
        return match bl {
            2 => '"',
            1 => '^',
            _ => '\'',
        };
    }
    if bot >= 2 && top == 0 {
        return match bl {
            2 => ',',
            1 => '.',
            _ => '.',
        };
    }

    if left >= 2 && right == 0 {
        return match bl {
            2 => '(',
            1 => '(',
            _ => ':',
        };
    }
    if right >= 2 && left == 0 {
        return match bl {
            2 => ')',
            1 => ')',
            _ => ':',
        };
    }

    if count >= 6 {
        return match bl {
            2 => '%',
            1 => '*',
            _ => '+',
        };
    }
    if count >= 5 {
        return match bl {
            2 => '*',
            1 => '+',
            _ => ':',
        };
    }

    if mid_c && count <= 3 {
        return match bl {
            2 => 'o',
            1 => '*',
            _ => '.',
        };
    }

    if top_r && bot_l && count <= 3 {
        return match bl {
            2 => '/',
            1 => '/',
            _ => '.',
        };
    }
    if top_l && bot_r && count <= 3 {
        return match bl {
            2 => '\\',
            1 => '\\',
            _ => '.',
        };
    }

    if count == 1 {
        if bot_c || bot_l || bot_r {
            return match bl {
                2 => '.',
                _ => '.',
            };
        }
        if top_c || top_l || top_r {
            return match bl {
                2 => '\'',
                1 => '\'',
                _ => '.',
            };
        }
        return match bl {
            2 => ':',
            1 => '.',
            _ => '.',
        };
    }

    if count <= 3 {
        return match bl {
            2 => ':',
            1 => ':',
            _ => '.',
        };
    }

    match bl {
        2 => '+',
        1 => ':',
        _ => '.',
    }
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h2 = h / 60.0;
    let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h2 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bufs(sw: usize, sh: usize) -> (Vec<bool>, Vec<f32>, Vec<f32>) {
        (
            vec![false; sw * sh],
            vec![0.0; sw * sh],
            vec![0.0; sw * sh],
        )
    }

    /// Same elapsed, same inputs → bit-identical buffers. The deleted
    /// samplers were vendored, so their guarantee was parity with jcode;
    /// `sample_signal` is original operant code, so the guarantee to pin is
    /// determinism: identical bits for identical calls, at any opt level
    /// (Rust never enables fast-math, so f32 accumulation is replay-stable).
    #[test]
    fn sample_signal_is_bit_identical_across_calls() {
        for &(sw, sh) in &[(120usize, 60usize), (90, 36), (108, 42), (57, 23)] {
            let (mut hit_a, mut lum_a, mut z_a) = bufs(sw, sh);
            let (mut hit_b, mut lum_b, mut z_b) = bufs(sw, sh);
            for &elapsed in &[0.0f32, 0.4, 0.8, 1.6, 2.4, 3.3, 5.7, 9.1] {
                sample_signal(elapsed, sw, sh, &mut hit_a, &mut lum_a, &mut z_a);
                sample_signal(elapsed, sw, sh, &mut hit_b, &mut lum_b, &mut z_b);
                assert_eq!(hit_a, hit_b);
                for i in 0..sw * sh {
                    assert_eq!(
                        lum_a[i].to_bits(),
                        lum_b[i].to_bits(),
                        "lum[{i}] mismatch at {sw}x{sh} t={elapsed}"
                    );
                    assert_eq!(
                        z_a[i].to_bits(),
                        z_b[i].to_bits(),
                        "z[{i}] mismatch at {sw}x{sh} t={elapsed}"
                    );
                }
            }
        }
    }

    /// The beam head must lead the afterglow: at t=0 the head sits at angle
    /// 0 (east) having just swept the cells clockwise of it, so a mid-radius
    /// cell just behind the head must be far brighter than one a quarter-turn
    /// back at the same radius.
    #[test]
    fn beam_head_leads_the_trail() {
        let (mut hit, mut lum, mut z) = bufs(90, 30);
        sample_signal(0.0, 90, 30, &mut hit, &mut lum, &mut z);
        let just_behind = 14 * 90 + 68; // angle ~ -0.064 (clockwise of east)
        let quarter_back = 22 * 90 + 45; // angle ~ +pi/2, a quarter-turn back
        assert!(
            z[just_behind] > z[quarter_back],
            "just_behind {} vs quarter_back {}",
            z[just_behind],
            z[quarter_back]
        );
    }

    /// Animates: a fixed subpixel must change intensity as the beam sweeps
    /// past it, else the idle screen is a static image.
    #[test]
    fn sweeps_over_time() {
        let (mut hit, mut lum, mut z) = bufs(90, 30);
        let idx = 15 * 90 + 80;
        sample_signal(0.0, 90, 30, &mut hit, &mut lum, &mut z);
        let at_zero = z[idx];
        // The head at t=0 is at angle 0; half a sweep later the east-rim
        // subpixel sits deep in the afterglow.
        sample_signal(
            std::f32::consts::TAU / BEAM_SPEED / 2.0,
            90,
            30,
            &mut hit,
            &mut lum,
            &mut z,
        );
        assert_ne!(at_zero, z[idx]);
    }
}
