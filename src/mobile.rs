// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full terms.

//! Mobile-Use guard helpers.
//!
//! These are deliberately **platform-agnostic** and operate on the JSON types
//! from [`crate::sdk::plugin`]. The host (Android/iOS) owns the actual
//! Accessibility/Shizuku/Root calls; these helpers encode the rules that keep
//! actions safe and reproducible:
//!
//! * node fingerprint comparison (stale target → refuse, do not replay);
//! * backend priority (Root → Shizuku → Accessibility);
//! * bounded gestures (≤10 fingers, ≤10s);
//! * bounded click perturbation (≤5% / 4px, keep centre for tiny targets);
//! * sensitive-field redaction.

use crate::sdk::plugin::{UiBounds, UiFingerprint, UiNode};

pub const MAX_GESTURE_FINGERS: u32 = 10;
pub const MAX_GESTURE_MS: u64 = 10_000;

fn bounds_eq(a: &UiBounds, b: &UiBounds) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.width - b.width).abs() < 0.5
        && (a.height - b.height).abs() < 0.5
}

/// Compare the stable identity of a UI node.
///
/// `tree_revision` is intentionally **ignored**: an unrelated tree change must
/// not reject an otherwise-identical node. Package/window/display/size/
/// orientation and the node's own identity/attributes must match.
pub fn fingerprint_matches(expected: &UiFingerprint, current: &UiFingerprint) -> bool {
    expected.package == current.package
        && expected.window_id == current.window_id
        && expected.display_id == current.display_id
        && expected.width == current.width
        && expected.height == current.height
        && expected.orientation == current.orientation
        && expected.view_id == current.view_id
        && expected.text == current.text
        && expected.description == current.description
        && bounds_eq(&expected.bounds, &current.bounds)
        && expected.enabled == current.enabled
        && expected.checked == current.checked
        && expected.actions == current.actions
}

/// Pick the best ready backend: `accessibility` → `shizuku` → `root`.
///
/// The host decides what is actually ready; this only encodes the priority and
/// never escalates on failure.
pub fn choose_backend(ready: &[String]) -> Option<&'static str> {
    for name in ["accessibility", "shizuku", "root"] {
        if ready.iter().any(|r| r.eq_ignore_ascii_case(name)) {
            return Some(name);
        }
    }
    None
}

/// Validate a gesture before it reaches the host.
pub fn validate_gesture(
    fingers: u32,
    duration_ms: u64,
    points: &[(f64, f64)],
) -> Result<(), String> {
    if !(1..=MAX_GESTURE_FINGERS).contains(&fingers) {
        return Err(format!(
            "gesture: fingers must be 1..={MAX_GESTURE_FINGERS} (got {fingers})"
        ));
    }
    if duration_ms == 0 || duration_ms > MAX_GESTURE_MS {
        return Err(format!(
            "gesture: duration must be 1..={MAX_GESTURE_MS}ms (got {duration_ms})"
        ));
    }
    if points.is_empty() || points.len() > MAX_GESTURE_FINGERS as usize {
        return Err("gesture: need 1..=10 points".to_string());
    }
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return Err("gesture: non-finite coordinate".to_string());
    }
    Ok(())
}

fn lcg(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    // Top 32 bits → [0,1).
    ((*state >> 32) as u32 as f64) / (u32::MAX as f64 + 1.0)
}

/// Deterministic, bounded click point inside `rect`.
///
/// Offset per axis is at most 5% of that axis (rounded down) and at most 4px;
/// axes smaller than 20px stay centred. The result is clamped inside the rect.
pub fn bounded_click(rect: &UiBounds, seed: u64) -> (f64, f64) {
    let cx = rect.x + rect.width / 2.0;
    let cy = rect.y + rect.height / 2.0;
    let max_dx = if rect.width < 20.0 {
        0.0
    } else {
        (rect.width * 0.05).floor().min(4.0)
    };
    let max_dy = if rect.height < 20.0 {
        0.0
    } else {
        (rect.height * 0.05).floor().min(4.0)
    };
    if max_dx == 0.0 && max_dy == 0.0 {
        return (cx, cy);
    }
    let mut state = seed | 1;
    let dx = (lcg(&mut state) * 2.0 - 1.0) * max_dx;
    let dy = (lcg(&mut state) * 2.0 - 1.0) * max_dy;
    (
        (cx + dx).clamp(rect.x, rect.x + rect.width),
        (cy + dy).clamp(rect.y, rect.y + rect.height),
    )
}

/// Redact a sensitive node's text/description before it reaches the model.
pub fn redact_sensitive(node: &mut UiNode) {
    if node.sensitive {
        node.text = "***".to_string();
        node.description = "***".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> UiFingerprint {
        UiFingerprint {
            package: "com.example".into(),
            window_id: 3,
            display_id: 0,
            width: 1080,
            height: 2400,
            orientation: 1,
            tree_revision: 10,
            view_id: "btn".into(),
            text: "Submit".into(),
            description: "".into(),
            bounds: UiBounds {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            },
            enabled: true,
            checked: None,
            actions: vec!["click".into()],
        }
    }

    #[test]
    fn fingerprint_ignores_tree_revision_but_not_node_changes() {
        let mut now = fp();
        now.tree_revision = 99;
        assert!(
            fingerprint_matches(&fp(), &now),
            "tree revision is informational"
        );
        now.text = "Cancel".into();
        assert!(!fingerprint_matches(&fp(), &now), "text change must reject");
        let mut moved = fp();
        moved.bounds.x = 12.0;
        assert!(
            !fingerprint_matches(&fp(), &moved),
            "bounds change must reject"
        );
        let mut disabled = fp();
        disabled.enabled = false;
        assert!(!fingerprint_matches(&fp(), &disabled));
    }

    #[test]
    fn backend_priority_is_accessibility_shizuku_root() {
        assert_eq!(
            choose_backend(&["accessibility".into(), "shizuku".into(), "root".into()]),
            Some("accessibility")
        );
        assert_eq!(
            choose_backend(&["shizuku".into(), "root".into()]),
            Some("shizuku")
        );
        assert_eq!(choose_backend(&["root".into()]), Some("root"));
        assert_eq!(choose_backend(&[]), None);
        assert_eq!(
            choose_backend(&["Shizuku".into(), "ACCESSIBILITY".into()]),
            Some("accessibility")
        );
    }

    #[test]
    fn gesture_limits_are_enforced() {
        assert!(validate_gesture(1, 100, &[(1.0, 2.0)]).is_ok());
        assert!(validate_gesture(10, 10_000, &[(1.0, 2.0)]).is_ok());
        assert!(validate_gesture(0, 100, &[(1.0, 2.0)]).is_err());
        assert!(validate_gesture(11, 100, &[(1.0, 2.0)]).is_err());
        assert!(validate_gesture(1, 0, &[(1.0, 2.0)]).is_err());
        assert!(validate_gesture(1, 10_001, &[(1.0, 2.0)]).is_err());
        assert!(validate_gesture(1, 100, &[]).is_err());
        assert!(validate_gesture(1, 100, &[(f64::NAN, 1.0)]).is_err());
    }

    #[test]
    fn bounded_click_stays_inside_and_centres_tiny_targets() {
        let rect = UiBounds {
            x: 100.0,
            y: 200.0,
            width: 100.0,
            height: 40.0,
        };
        for seed in 1..=50 {
            let (x, y) = bounded_click(&rect, seed);
            assert!(x >= rect.x && x <= rect.x + rect.width, "x={x}");
            assert!(y >= rect.y && y <= rect.y + rect.height, "y={y}");
        }
        let tiny = UiBounds {
            x: 5.0,
            y: 6.0,
            width: 10.0,
            height: 12.0,
        };
        assert_eq!(bounded_click(&tiny, 42), (10.0, 12.0));
    }

    #[test]
    fn sensitive_nodes_are_redacted() {
        let mut n = UiNode {
            text: "hunter2".into(),
            description: "password".into(),
            sensitive: true,
            ..Default::default()
        };
        redact_sensitive(&mut n);
        assert_eq!(n.text, "***");
        assert_eq!(n.description, "***");
        let mut plain = UiNode {
            text: "hello".into(),
            sensitive: false,
            ..Default::default()
        };
        redact_sensitive(&mut plain);
        assert_eq!(plain.text, "hello");
    }
}
