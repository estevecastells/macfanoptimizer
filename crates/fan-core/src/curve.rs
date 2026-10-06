//! Piecewise-linear temperature → fan-duty curve.

use serde::{Deserialize, Serialize};

/// A fan curve: `(temperature °C, duty %)` points, sorted by temperature.
///
/// Duty is a percentage of the fan's controllable range: 0 % = the fan's
/// minimum RPM, 100 % = its maximum RPM. Below the first point the curve
/// yields 0 %, above the last point it yields the last point's duty.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<(f64, f64)>", into = "Vec<(f64, f64)>")]
pub struct Curve {
    points: Vec<(f64, f64)>,
}

impl Curve {
    pub fn new(points: Vec<(f64, f64)>) -> Result<Self, String> {
        if points.is_empty() {
            return Err("curve needs at least one point".into());
        }
        for w in points.windows(2) {
            if w[1].0 <= w[0].0 {
                return Err(format!("curve temperatures must strictly increase ({} then {})", w[0].0, w[1].0));
            }
            if w[1].1 < w[0].1 {
                return Err(format!("curve duty must not decrease ({}% at {}°C then {}%)", w[0].1, w[0].0, w[1].1));
            }
        }
        if let Some(p) = points.iter().find(|p| !(0.0..=100.0).contains(&p.1) || !p.0.is_finite()) {
            return Err(format!("invalid curve point {p:?}: duty must be 0–100"));
        }
        Ok(Curve { points })
    }

    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// Temperature at which the curve first asks for any cooling.
    pub fn start_temp(&self) -> f64 {
        self.points.iter().find(|p| p.1 > 0.0).map_or(f64::INFINITY, |p| p.0)
    }

    /// Duty (0–100) for a temperature.
    pub fn duty(&self, temp: f64) -> f64 {
        let pts = &self.points;
        if temp < pts[0].0 {
            return 0.0;
        }
        for w in pts.windows(2) {
            let ((t0, d0), (t1, d1)) = (w[0], w[1]);
            if temp <= t1 {
                return d0 + (d1 - d0) * (temp - t0) / (t1 - t0);
            }
        }
        pts[pts.len() - 1].1
    }
}

impl TryFrom<Vec<(f64, f64)>> for Curve {
    type Error = String;
    fn try_from(v: Vec<(f64, f64)>) -> Result<Self, String> {
        Curve::new(v)
    }
}

impl From<Curve> for Vec<(f64, f64)> {
    fn from(c: Curve) -> Self {
        c.points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c() -> Curve {
        Curve::new(vec![(60.0, 0.0), (70.0, 20.0), (80.0, 60.0), (90.0, 100.0)]).unwrap()
    }

    #[test]
    fn interpolates() {
        let c = c();
        assert_eq!(c.duty(40.0), 0.0);
        assert_eq!(c.duty(60.0), 0.0);
        assert_eq!(c.duty(65.0), 10.0);
        assert_eq!(c.duty(75.0), 40.0);
        assert_eq!(c.duty(90.0), 100.0);
        assert_eq!(c.duty(120.0), 100.0);
    }

    #[test]
    fn start_temp_is_first_nonzero_point() {
        assert_eq!(c().start_temp(), 70.0);
    }

    #[test]
    fn rejects_bad_curves() {
        assert!(Curve::new(vec![]).is_err());
        assert!(Curve::new(vec![(70.0, 0.0), (60.0, 10.0)]).is_err());
        assert!(Curve::new(vec![(60.0, 50.0), (70.0, 10.0)]).is_err());
        assert!(Curve::new(vec![(60.0, 150.0)]).is_err());
    }

    #[test]
    fn serde_roundtrip_validates() {
        let json = serde_json::to_string(&c()).unwrap();
        assert_eq!(json, "[[60.0,0.0],[70.0,20.0],[80.0,60.0],[90.0,100.0]]");
        assert!(serde_json::from_str::<Curve>("[[70,0],[60,10]]").is_err());
    }
}
