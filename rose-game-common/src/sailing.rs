//! Shared sailing physics used by both the client (prediction) and the server
//! (validation/authority). Keeping this in one crate prevents client/server drift.

use bevy::math::Vec2;

/// Wraps any angle to the `0..TAU` range.
pub fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

/// Returns the shortest signed angular delta from `from` to `to`.
pub fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    let mut diff = (to - from).rem_euclid(std::f32::consts::TAU);
    if diff > std::f32::consts::PI {
        diff -= std::f32::consts::TAU;
    }
    diff
}

/// Absolute wind-relative angle in radians, folded into `0..PI`.
pub fn angle_to_wind_abs(heading: f32, wind_angle: f32) -> f32 {
    let angle = normalize_angle(heading - wind_angle);
    if angle > std::f32::consts::PI {
        std::f32::consts::TAU - angle
    } else {
        angle
    }
}

/// ROSE sailing polar curve. Returns a speed factor in the `0..1` range.
pub fn sail_speed_factor(angle_to_wind: f32) -> f32 {
    let angle = angle_to_wind.abs().min(std::f32::consts::PI);
    if angle < 0.78 {
        (angle / 0.78).powf(2.0) * 0.3
    } else if angle < 1.57 {
        let t = (angle - 0.78) / (1.57 - 0.78);
        0.3 + t * 0.7
    } else if angle < 2.36 {
        let t = (angle - 1.57) / (2.36 - 1.57);
        1.0 - t * 0.2
    } else {
        let t = (angle - 2.36) / (std::f32::consts::PI - 2.36);
        0.8 - t * 0.3
    }
}

pub fn optimal_sail_trim(angle_to_wind_abs: f32) -> f32 {
    (angle_to_wind_abs * 0.5).clamp(0.0, std::f32::consts::PI)
}

pub fn trim_efficiency(current_trim: f32, optimal_trim: f32) -> f32 {
    (1.0 - ((current_trim - optimal_trim).abs() / std::f32::consts::PI)).clamp(0.1, 1.0)
}

pub fn lerp_angle(from: f32, to: f32, t: f32) -> f32 {
    normalize_angle(from + shortest_angle_delta(from, to) * t.clamp(0.0, 1.0))
}

#[derive(Debug, Clone, Copy)]
pub struct SailingStepInput {
    pub heading: f32,
    pub speed: f32,
    /// Rudder input in `-1..1` (left/right).
    pub rudder: f32,
    /// Throttle input in `-1..1`: `+1` full throttle, `-1` brake, `0` cruise.
    pub throttle: f32,
    pub max_speed: f32,
    /// Wind angle in radians (0 = +Y in Position space).
    pub wind_angle: f32,
    /// Wind speed in m/s. Only used as a mild speed modifier.
    pub wind_speed: f32,
    pub dt: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct SailingStepOutput {
    pub heading: f32,
    pub speed: f32,
    pub forward_cm: Vec2,
}

/// Wind speed at which the wind modifier reaches its full effect (m/s).
pub const WIND_REFERENCE_SPEED: f32 = 5.0;
/// Maximum speed multiplier from a direct tailwind (+30%).
pub const WIND_MAX_SPEED_EFFECT: f32 = 0.3;

/// Advances the boat's pure driving state (direct-drive model).
///
/// The boat moves where it is pointed and turns with the rudder. Wind does
/// not change the heading or handling; it only speeds the boat up a little
/// when going with the wind and slows it down a little when going against it.
/// Position/water/collision remain caller-owned.
pub fn sailing_step(input: SailingStepInput) -> SailingStepOutput {
    let max_speed = input.max_speed.max(0.1);
    let dt = input.dt.max(0.0);

    let turn_rate = 1.2 * (0.4 + 0.6 * (input.speed / max_speed).clamp(0.0, 1.0));
    let heading = normalize_angle(input.heading + input.rudder.clamp(-1.0, 1.0) * turn_rate * dt);

    let throttle = input.throttle.clamp(-1.0, 1.0);
    let base_target = max_speed * (0.5 + 0.5 * throttle);

    let forward = Vec2::new(heading.sin(), heading.cos());
    let wind_direction = Vec2::new(input.wind_angle.sin(), input.wind_angle.cos());
    let wind_along = forward.dot(wind_direction);
    let wind_factor = 1.0
        + wind_along
            * WIND_MAX_SPEED_EFFECT
            * (input.wind_speed / WIND_REFERENCE_SPEED).clamp(0.0, 1.5);

    let target_speed = (base_target * wind_factor).clamp(0.0, max_speed * 1.15);
    let accel = if target_speed > input.speed { 2.0 } else { 1.5 };
    // Exponential approach is stable for any dt (no overshoot under frame-rate spikes).
    let blend = 1.0 - (-accel * dt).exp();
    let speed =
        (input.speed + (target_speed - input.speed) * blend).clamp(0.0, max_speed * 1.15);

    SailingStepOutput {
        heading,
        speed,
        forward_cm: forward * speed * dt * 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polar_curve_has_expected_shape() {
        assert!(sail_speed_factor(0.0) < 0.01);
        assert!(sail_speed_factor(std::f32::consts::FRAC_PI_2) > 0.95);
        assert!(sail_speed_factor(std::f32::consts::PI) < 0.55);
    }

    #[test]
    fn angle_interpolation_uses_shortest_arc() {
        let from = 350.0_f32.to_radians();
        let to = 10.0_f32.to_radians();
        let mid = lerp_angle(from, to, 0.5).to_degrees();
        assert!(mid < 1.0 || mid > 359.0);
    }

    #[test]
    fn drive_model_moves_straight_along_heading() {
        let output = sailing_step(SailingStepInput {
            heading: 0.0,
            speed: 4.0,
            rudder: 0.0,
            throttle: 1.0,
            max_speed: 9.0,
            wind_angle: 0.0,
            wind_speed: 0.0,
            dt: 1.0,
        });

        assert!(output.forward_cm.y > 0.0);
        assert!(output.forward_cm.x.abs() < 0.001);
        assert!((output.heading - 0.0).abs() < 0.001);
    }

    #[test]
    fn drive_model_turns_with_rudder() {
        let straight = sailing_step(SailingStepInput {
            heading: 0.0,
            speed: 4.0,
            rudder: 0.0,
            throttle: 1.0,
            max_speed: 9.0,
            wind_angle: 0.0,
            wind_speed: 0.0,
            dt: 1.0,
        });
        let turned = sailing_step(SailingStepInput {
            heading: 0.0,
            speed: 4.0,
            rudder: 1.0,
            throttle: 1.0,
            max_speed: 9.0,
            wind_angle: 0.0,
            wind_speed: 0.0,
            dt: 1.0,
        });

        assert!(turned.heading > straight.heading);
    }

    #[test]
    fn idle_throttle_cruises_at_half_speed() {
        let mut output = sailing_step(SailingStepInput {
            heading: 0.0,
            speed: 0.0,
            rudder: 0.0,
            throttle: 0.0,
            max_speed: 9.0,
            wind_angle: 0.0,
            wind_speed: 0.0,
            dt: 1.0 / 60.0,
        });
        for _ in 0..600 {
            output = sailing_step(SailingStepInput {
                heading: output.heading,
                speed: output.speed,
                rudder: 0.0,
                throttle: 0.0,
                max_speed: 9.0,
                wind_angle: 0.0,
                wind_speed: 0.0,
                dt: 1.0 / 60.0,
            });
        }

        assert!((output.speed - 4.5).abs() < 0.1);
    }

    #[test]
    fn wind_speed_modifier_is_mild() {
        let step = |wind_angle: f32| {
            let mut output = sailing_step(SailingStepInput {
                heading: 0.0,
                speed: 0.0,
                rudder: 0.0,
                throttle: 1.0,
                max_speed: 9.0,
                wind_angle,
                wind_speed: 5.0,
                dt: 1.0 / 60.0,
            });
            for _ in 0..600 {
                output = sailing_step(SailingStepInput {
                    heading: output.heading,
                    speed: output.speed,
                    rudder: 0.0,
                    throttle: 1.0,
                    max_speed: 9.0,
                    wind_angle,
                    wind_speed: 5.0,
                    dt: 1.0 / 60.0,
                });
            }
            output
        };

        let tail = step(0.0);
        let calm = step(std::f32::consts::FRAC_PI_2);
        let head = step(std::f32::consts::PI);

        assert!(tail.speed > calm.speed);
        assert!(head.speed < calm.speed);
        assert!(tail.speed < calm.speed * 1.4, "wind effect should be small");
        assert!(head.speed > calm.speed * 0.6, "wind effect should be small");
    }
}
