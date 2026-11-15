//! Tasmota lights: state from STATE/RESULT messages and the colour maths the front ends share.
//!
//! The light type follows the number of PWM channels (Tasmota docs, Lights):
//! 1 dimmer, 2 CCT (cold/warm white), 3 RGB, 4 RGBW, 5 RGBCCT. Commands: `Dimmer` 0..100,
//! `HSBColor h,s,b` (0..359, 0..100, 0..100), `CT` 153..500 mireds, `White` 0..100.

use serde::Serialize;
use serde_json::Value;

pub const CT_MIN: i64 = 153;
pub const CT_MAX: i64 = 500;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Light {
    /// PWM channels: 1..=5.
    pub channels: u8,
    pub dimmer: Option<i64>,
    /// Hue 0..359, saturation 0..100, brightness 0..100.
    pub hsb: Option<(i64, i64, i64)>,
    pub ct: Option<i64>,
    pub white: Option<i64>,
    /// Hex as Tasmota reports it, e.g. "FF8000" or "FF800000FF".
    pub color: Option<String>,
}

impl Light {
    pub fn has_color(&self) -> bool {
        self.channels >= 3
    }

    /// CT slider: CCT lights and RGBCCT.
    pub fn has_ct(&self) -> bool {
        self.channels == 2 || self.channels == 5 || self.ct.is_some() && self.channels != 3
    }

    /// Separate white slider: RGBW and RGBCCT.
    pub fn has_white(&self) -> bool {
        self.channels >= 4
    }

    pub fn kind(&self) -> &'static str {
        match self.channels {
            1 => "Dimmer",
            2 => "CCT",
            3 => "RGB",
            4 => "RGBW",
            5 => "RGBCCT",
            _ => "Light",
        }
    }

    /// The current colour as RGB for a swatch: from the RGB channels, or a white tone.
    pub fn swatch(&self) -> (u8, u8, u8) {
        let white_mode = self.has_white()
            && self.white.unwrap_or(0) > 0
            && self.color.as_deref().and_then(hex_rgb) == Some((0, 0, 0));
        if self.has_color() && !white_mode {
            if let Some((h, s, _)) = self.hsb {
                return hsv_to_rgb(h as f32, s as f32 / 100.0, 1.0);
            }
            if let Some(c) = self.color.as_deref().and_then(hex_rgb) {
                return c;
            }
        }
        ct_rgb(self.ct.unwrap_or(326))
    }

    /// Reads the light fields of a STATE or RESULT object. Returns true if any was there.
    pub fn update(&mut self, v: &Value) -> bool {
        let mut seen = false;
        if let Some(d) = v.get("Dimmer").and_then(Value::as_i64) {
            self.dimmer = Some(d);
            seen = true;
        }
        if let Some(h) = v.get("HSBColor").and_then(Value::as_str) {
            let p: Vec<i64> = h.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if p.len() == 3 {
                self.hsb = Some((p[0], p[1], p[2]));
                seen = true;
            }
        }
        if let Some(ct) = v.get("CT").and_then(Value::as_i64) {
            self.ct = Some(ct);
            seen = true;
        }
        if let Some(w) = v.get("White").and_then(Value::as_i64) {
            self.white = Some(w);
            seen = true;
        }
        if let Some(c) = v.get("Color").and_then(Value::as_str) {
            let hex = c.trim();
            if hex.len() >= 2 && hex.len() % 2 == 0 && hex.chars().all(|x| x.is_ascii_hexdigit()) {
                self.channels = (hex.len() / 2).min(5) as u8;
                self.color = Some(hex.to_ascii_uppercase());
                seen = true;
            } else if hex.contains(',') {
                // Old firmware: "255,128,0"
                self.channels = hex.split(',').count().clamp(1, 5) as u8;
                seen = true;
            }
        }
        if let Some(ch) = v.get("Channel").and_then(Value::as_array)
            && !ch.is_empty()
        {
            self.channels = ch.len().min(5) as u8;
            seen = true;
        }
        if seen && self.channels == 0 {
            // Only Dimmer reported: a single channel light.
            self.channels = if self.ct.is_some() { 2 } else { 1 };
        }
        seen
    }
}

/// "FF8000" (first 6 digits) → (255, 128, 0).
pub fn hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    if hex.len() < 6 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some((b(0)?, b(2)?, b(4)?))
}

/// h 0..360, s and v 0..1.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// Rough colour of a white light at `ct` mireds (153 cold … 500 warm), for swatches.
pub fn ct_rgb(ct: i64) -> (u8, u8, u8) {
    let t = ((ct.clamp(CT_MIN, CT_MAX) - CT_MIN) as f32) / ((CT_MAX - CT_MIN) as f32);
    let lerp = |a: f32, b: f32| (a + (b - a) * t).round() as u8;
    (lerp(205.0, 255.0), lerp(225.0, 180.0), lerp(255.0, 100.0))
}

/// Hue and saturation (0..100) of a point on a colour wheel: `dx`, `dy` from the centre,
/// `radius` of the wheel. Red at the right, hue growing counter-clockwise like Tasmota's
/// web UI does not matter here: the wheel only has to match what it draws (hsv_to_rgb).
pub fn wheel_pick(dx: f32, dy: f32, radius: f32) -> (i64, i64) {
    let hue = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
    let sat = ((dx * dx + dy * dy).sqrt() / radius).clamp(0.0, 1.0);
    (hue.round() as i64 % 360, (sat * 100.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_from_state() {
        let mut l = Light::default();
        let v: Value = serde_json::from_str(
            r#"{"POWER":"ON","Dimmer":60,"Color":"FF8000","HSBColor":"30,100,60","Channel":[100,50,0]}"#,
        )
        .unwrap();
        assert!(l.update(&v));
        assert_eq!(
            (l.channels, l.kind(), l.has_color(), l.has_ct()),
            (3, "RGB", true, false)
        );
        assert_eq!(l.hsb, Some((30, 100, 60)));

        let mut cct = Light::default();
        cct.update(
            &serde_json::from_str(r#"{"Dimmer":40,"Color":"8040","CT":326,"Channel":[50,25]}"#)
                .unwrap(),
        );
        assert_eq!(
            (cct.kind(), cct.has_ct(), cct.has_color()),
            ("CCT", true, false)
        );

        let mut rgbcct = Light::default();
        rgbcct.update(
            &serde_json::from_str(r#"{"Dimmer":100,"Color":"0000FFFF00","CT":200,"White":100}"#)
                .unwrap(),
        );
        assert_eq!(
            (rgbcct.kind(), rgbcct.has_ct(), rgbcct.has_white()),
            ("RGBCCT", true, true)
        );

        let mut none = Light::default();
        assert!(!none.update(&serde_json::from_str(r#"{"POWER":"ON"}"#).unwrap()));
    }

    #[test]
    fn colours() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), (255, 255, 255));
        assert_eq!(hex_rgb("FF8000"), Some((255, 128, 0)));
        assert_eq!(wheel_pick(10.0, 0.0, 10.0), (0, 100));
        assert_eq!(wheel_pick(0.0, -10.0, 10.0), (90, 100));
        assert_eq!(wheel_pick(0.0, 0.0, 10.0).1, 0);
    }
}
