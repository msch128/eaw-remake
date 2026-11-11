//! Numbers of the bar headers that stay inside their boxes.
//!
//! A read-out (credits, income, month, tech, population of the galactic and the tactical header) has a fixed box and, for the credit
//! figures, the credit symbol at its left edge. The symbol keeps its place and the box reserves its width; the number is right-aligned
//! in the rest. When the number is wider than that rest, the font shrinks in steps down to a readable minimum, and only below that
//! the number is abbreviated ("10,9M", then "11M"). The choice is a pure function of the candidates and of one measured font width, so
//! a figure that changes every second keeps its size and does not flicker.
//!
//! Estimate (no data): the steps of 4 % of the field's font size and the minimum of 80 % of it. The header's read-outs are 8.7
//! logical px, 12.3 px on a 1080p screen; the minimum is about 10 px there.

use bevy::prelude::*;
use bevy::text::{FontSize, TextLayoutInfo};
use bevy::ui::ComputedNode;

/// Smallest font as a fraction of the field's own size.
pub(crate) const MIN_SCALE: f32 = 0.8;
/// One shrink step as a fraction of the field's own size.
pub(crate) const STEP_SCALE: f32 = 0.04;

/// A header read-out that fits its box: the texts it may show, longest first, and what was measured of its font.
#[derive(Component, Debug, Clone, Default)]
pub struct FitText {
    /// The full text first, then shorter forms of the same value (abbreviations).
    pub candidates: Vec<String>,
    /// The field's own font size (logical px).
    pub base_px: f32,
    /// Width of one weighted character per logical font px, in physical px, as measured (the largest seen at this box width).
    em: f32,
    /// Width of the box (physical px) the measurement belongs to.
    avail: f32,
    /// What this system wrote last: text and font size, the layout of the next frame measures it.
    shown: Option<(String, f32)>,
}

impl FitText {
    pub fn new(base_px: f32) -> Self {
        Self { base_px, ..default() }
    }

    /// Whether the box shows a shorter form than the full text (the hover card then names the full value).
    pub fn abbreviated(&self) -> bool {
        match (&self.shown, self.candidates.first()) {
            (Some((s, _)), Some(full)) => s != full,
            _ => false,
        }
    }
}

/// Width of a text in "digit widths": digits count 1, narrow marks less, capitals more. Only the ratio between two texts of the same
/// font matters; the measured width calibrates the rest.
pub(crate) fn weighted_len(s: &str) -> f32 {
    s.chars()
        .map(|c| match c {
            '0'..='9' => 1.0,
            '.' | ',' | ':' | ';' | '\'' | 'i' | 'l' | 'I' => 0.5,
            ' ' => 0.5,
            '/' | '-' | '+' => 0.7,
            'M' | 'W' | 'm' | 'w' => 1.4,
            c if c.is_uppercase() => 1.15,
            _ => 1.0,
        })
        .sum()
}

/// Shorter forms of a credit figure, most precise first: "264,2K" / "264K", "10,9M" / "11M", "1,2B" / "1B". Below ten thousand the
/// full figure has at most five characters and is not abbreviated.
pub(crate) fn abbreviations(v: i64) -> Vec<String> {
    let a = v.unsigned_abs() as f64;
    let sign = if v < 0 { "-" } else { "" };
    let (div, unit) = match a {
        a if a >= 1e9 => (1e9, 'B'),
        a if a >= 1e6 => (1e6, 'M'),
        a if a >= 1e4 => (1e3, 'K'),
        _ => return Vec::new(),
    };
    let x = a / div;
    let tenths = (x * 10.0).round() / 10.0;
    let mut out = Vec::new();
    if tenths.fract().abs() > 1e-9 && tenths < 1000.0 {
        out.push(format!("{sign}{}{unit}", format!("{tenths:.1}").replace('.', ",")));
    }
    out.push(format!("{sign}{:.0}{unit}", x.round()));
    out
}

/// The candidate and font size to show: the first candidate that fits at the largest size of the steps, trying every size of one
/// candidate before the next one. `em` is the measured width of a weighted character per font px (0: not measured yet, the full
/// text at full size), `avail` the box width in the same unit.
pub(crate) fn choose(candidates: &[String], em: f32, avail: f32, base_px: f32) -> (usize, f32) {
    if candidates.is_empty() || em <= 0.0 || avail <= 0.0 {
        return (0, base_px);
    }
    let steps = ((1.0 - MIN_SCALE) / STEP_SCALE).round() as u32;
    for (i, c) in candidates.iter().enumerate() {
        let w = weighted_len(c) * em;
        for k in 0..=steps {
            let px = base_px * (1.0 - STEP_SCALE * k as f32).max(MIN_SCALE);
            if w * px <= avail {
                return (i, px);
            }
        }
    }
    (candidates.len() - 1, base_px * MIN_SCALE)
}

/// Fits every header read-out into its box after `bound_fields` wrote the full texts.
pub(crate) fn fit_fields(mut q: Query<(&mut FitText, &mut Text, &mut TextFont, &TextLayoutInfo, &ChildOf)>, boxes: Query<&ComputedNode>) {
    for (mut fit, mut text, mut font, layout, parent) in &mut q {
        let Ok(holder) = boxes.get(parent.parent()) else { continue };
        let avail = holder.content_box().width();
        if avail <= 0.0 || fit.candidates.is_empty() {
            continue;
        }
        if (avail - fit.avail).abs() > 0.5 {
            fit.avail = avail;
            fit.em = 0.0;
        }
        // The layout of this frame shows what was written last frame. The width per character only grows, so two candidates that
        // measure a little differently cannot take turns.
        if let Some((s, px)) = &fit.shown {
            let wl = weighted_len(s);
            if wl > 0.0 && *px > 0.0 && layout.size.x > 0.0 {
                // The layout size is in logical px, the box in physical px.
                fit.em = fit.em.max(layout.size.x * layout.scale_factor.max(1e-3) / (px * wl));
            }
        }
        let (i, px) = choose(&fit.candidates, fit.em, avail, fit.base_px);
        let s = fit.candidates[i].clone();
        if text.0 != s {
            text.0.clone_from(&s);
        }
        if font.font_size != FontSize::Px(px) {
            font.font_size = FontSize::Px(px);
        }
        fit.shown = Some((s, px));
    }
}

/// Test hook (`--hud-figures` of an offscreen run): header figures shown instead of the game's, so a capture can show a treasury or an
/// income of any size. Display only: the game's own numbers are untouched.
#[derive(Resource, Debug, Clone, Default)]
pub struct FigureOverride {
    pub credits: Option<i64>,
    pub month_income: Option<i64>,
    pub planet_income: Option<i64>,
    pub pop: Option<(u32, u32)>,
}

impl FigureOverride {
    /// The bar data with these figures in place of the game's.
    pub(crate) fn apply(&self, d: &crate::CommandBarData) -> crate::CommandBarData {
        let mut d = d.clone();
        d.credits = self.credits.unwrap_or(d.credits);
        d.month_income = self.month_income.unwrap_or(d.month_income);
        d.planet_income = self.planet_income.unwrap_or(d.planet_income);
        (d.pop_current, d.pop_cap) = self.pop.unwrap_or((d.pop_current, d.pop_cap));
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn large_figures_get_short_forms() {
        assert_eq!(abbreviations(10_881_808), c(&["10,9M", "11M"]));
        assert_eq!(abbreviations(-264_171), c(&["-264,2K", "-264K"]));
        assert_eq!(abbreviations(40_000), c(&["40K"]));
        assert_eq!(abbreviations(1_250_000_000), c(&["1,3B", "1B"]));
        assert!(abbreviations(9_999).is_empty());
        assert!(abbreviations(-4_634).is_empty());
    }

    #[test]
    fn a_figure_that_fits_keeps_full_size_and_text() {
        // 1 font px per weighted character, a box of 100 px, a 14.5 px font: "4634" is 58 px wide.
        assert_eq!(choose(&c(&["4634"]), 1.0, 100.0, 14.5), (0, 14.5));
        // Not measured yet: the full text at full size.
        assert_eq!(choose(&c(&["10.881.808", "10,9M"]), 0.0, 10.0, 14.5), (0, 14.5));
    }

    #[test]
    fn the_font_shrinks_before_the_figure_is_abbreviated() {
        let cands = c(&["10.881.808", "10,9M", "11M"]);
        let base = 14.5;
        let full = weighted_len(&cands[0]);
        // A box 10 % too narrow for the full figure: a smaller font, the full figure.
        let (i, px) = choose(&cands, 1.0, full * base * 0.9, base);
        assert_eq!(i, 0);
        assert!(px < base && px >= base * MIN_SCALE && full * px <= full * base * 0.9, "{px}");
        // Too narrow even at the minimum size: the abbreviation, as large as it fits.
        let (i, px) = choose(&cands, 1.0, full * base * MIN_SCALE * 0.9, base);
        assert_eq!(i, 1);
        assert!(weighted_len(&cands[1]) * px <= full * base * MIN_SCALE * 0.9);
        // Nothing fits: the shortest form at the minimum size.
        assert_eq!(choose(&cands, 1.0, 1.0, base), (2, base * MIN_SCALE));
    }

    #[test]
    fn the_chosen_text_never_leaves_its_box() {
        let cands = [vec!["-264.171".to_owned()], abbreviations(-264_171)].concat();
        for avail in (20..200).map(|w| w as f32) {
            let (i, px) = choose(&cands, 0.62, avail, 14.5);
            let fits = weighted_len(&cands[i]) * 0.62 * px <= avail;
            // Only the last resort may overflow, and only when even it does not fit at the minimum size.
            assert!(fits || (i == cands.len() - 1 && px == 14.5 * MIN_SCALE), "{avail}: {i} {px}");
        }
    }
}
