//! Animation math: easing, a critically damped spring, the panel's
//! openness state machine, the toast stack's enter/move/leave motion and
//! the panel groups' expand/collapse morph.
//!
//! Everything takes explicit `Instant`s so it is deterministic under test;
//! the Wayland layer feeds frame-callback times in.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tidings_core::config::AnimationConfig;
use tidings_core::model::NotificationId;

/// Cubic ease-out: fast start, gentle stop.
#[must_use]
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Panel content opacity for a given openness: rises quickly so a half-open
/// panel is mostly opaque, while still fading fully to zero.
#[must_use]
pub fn panel_opacity(openness: f32) -> f32 {
    let o = openness.clamp(0.0, 1.0);
    1.0 - (1.0 - o) * (1.0 - o)
}

/// Critically damped spring towards a target, integrated in closed form so
/// the result is frame-rate independent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    /// Natural angular frequency (1/s). Higher settles faster.
    pub omega: f32,
}

impl Spring {
    /// Panel spring: visually done in ~250 ms, settled in ~350 ms for a
    /// full-width move from rest.
    pub const PANEL_OMEGA: f32 = 24.0;
    /// Settle thresholds (openness units and units/s).
    const EPS_X: f32 = 0.003;
    const EPS_V: f32 = 0.05;

    #[must_use]
    pub fn new(value: f32, velocity: f32, target: f32, omega: f32) -> Self {
        Self {
            value,
            velocity,
            target,
            omega,
        }
    }

    /// Advances by `dt` seconds. Returns true when settled (value snapped).
    pub fn step(&mut self, dt: f32) -> bool {
        let w = self.omega;
        let a = self.value - self.target;
        let b = self.velocity + w * a;
        let e = (-w * dt).exp();
        let disp = (a + b * dt) * e;
        self.velocity = (b - w * (a + b * dt)) * e;
        self.value = self.target + disp;
        if disp.abs() < Self::EPS_X && self.velocity.abs() < Self::EPS_V {
            self.value = self.target;
            self.velocity = 0.0;
            true
        } else {
            false
        }
    }
}

/// What the panel did in response to a step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionStep {
    pub openness: f32,
    /// `Some(open)` exactly once, when an `AnimateTo` finishes.
    pub settled: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    /// Finger tracking: `last` is the previous sample for velocity.
    Tracking {
        last: (Instant, f32),
    },
    Animating {
        spring: Spring,
        last: Instant,
    },
}

/// The panel's openness over time: `Track` jumps, `AnimateTo` springs, and
/// a `Track` interrupts any running animation.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelMotion {
    openness: f32,
    /// Smoothed tracking velocity (openness/s), handed to the spring.
    velocity: f32,
    phase: Phase,
    /// Spring stiffness; `None` snaps straight to the target.
    omega: Option<f32>,
}

impl Default for PanelMotion {
    fn default() -> Self {
        Self::new(Timing::default().panel_omega)
    }
}

impl PanelMotion {
    /// A closed panel springing with stiffness `omega` (`None`: no animation).
    #[must_use]
    pub fn new(omega: Option<f32>) -> Self {
        Self {
            openness: 0.0,
            velocity: 0.0,
            phase: Phase::Idle,
            omega,
        }
    }

    #[must_use]
    pub fn openness(&self) -> f32 {
        self.openness
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        matches!(self.phase, Phase::Animating { .. })
    }

    /// Jump to `x` (finger tracking). Cancels any animation.
    pub fn track(&mut self, x: f32, now: Instant) {
        let x = x.clamp(0.0, 1.0);
        if let Phase::Tracking { last: (t, prev) } = self.phase {
            let dt = now.saturating_duration_since(t).as_secs_f32();
            if dt > 0.0 && dt < 0.1 {
                let v = (x - prev) / dt;
                self.velocity = 0.5 * self.velocity + 0.5 * v;
            } else if dt >= 0.1 {
                self.velocity = 0.0;
            }
        } else {
            self.velocity = 0.0;
        }
        self.openness = x;
        self.phase = Phase::Tracking { last: (now, x) };
    }

    /// Start animating towards `target`, inheriting the finger's velocity.
    /// Returns a settle immediately if already at rest on the target.
    pub fn animate_to(&mut self, target: f32, now: Instant) -> Option<bool> {
        let target = target.clamp(0.0, 1.0);
        let velocity = match self.phase {
            Phase::Animating { spring, .. } => spring.velocity,
            // A stale finger velocity (no samples for a while) is zero.
            Phase::Tracking { last: (t, _) }
                if now.saturating_duration_since(t) < Duration::from_millis(100) =>
            {
                self.velocity
            }
            _ => 0.0,
        };
        let at_rest =
            (self.openness - target).abs() < f32::EPSILON && velocity.abs() < Spring::EPS_V;
        let Some(omega) = self.omega.filter(|_| !at_rest) else {
            self.openness = target;
            self.phase = Phase::Idle;
            return Some(target >= 0.5);
        };
        self.phase = Phase::Animating {
            spring: Spring::new(self.openness, velocity, target, omega),
            last: now,
        };
        None
    }

    /// Advance a running animation to `now`.
    pub fn step(&mut self, now: Instant) -> MotionStep {
        let mut settled = None;
        if let Phase::Animating { mut spring, last } = self.phase {
            // Cap the step so a stalled compositor doesn't make us jump.
            let dt = now.saturating_duration_since(last).as_secs_f32().min(0.05);
            let done = spring.step(dt);
            self.openness = spring.value.clamp(0.0, 1.0);
            if done {
                self.phase = Phase::Idle;
                settled = Some(spring.target >= 0.5);
            } else {
                self.phase = Phase::Animating { spring, last: now };
            }
        }
        MotionStep {
            openness: self.openness,
            settled,
        }
    }
}

/// Duration of a toast's enter/leave fade-slide.
pub const TOAST_ENTER: Duration = Duration::from_millis(260);
pub const TOAST_LEAVE: Duration = Duration::from_millis(200);
/// Duration of a toast gliding to its new slot.
pub const TOAST_MOVE: Duration = Duration::from_millis(240);
/// Horizontal travel of the enter/leave slide, in logical px.
pub const TOAST_SLIDE: f32 = 48.0;

/// How long every UI motion takes: the built-in durations, sped up or
/// slowed down (or switched off) by the user's [`AnimationConfig`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub toast_enter: Duration,
    pub toast_leave: Duration,
    pub toast_move: Duration,
    pub group_morph: Duration,
    /// Panel spring stiffness; `None` snaps.
    pub panel_omega: Option<f32>,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            toast_enter: TOAST_ENTER,
            toast_leave: TOAST_LEAVE,
            toast_move: TOAST_MOVE,
            group_morph: GROUP_MORPH,
            panel_omega: Some(Spring::PANEL_OMEGA),
        }
    }
}

impl Timing {
    /// The defaults at `config.speed`, or all instant when disabled.
    #[must_use]
    pub fn new(config: &AnimationConfig) -> Self {
        let base = Self::default();
        if !config.enabled {
            return Self {
                toast_enter: Duration::ZERO,
                toast_leave: Duration::ZERO,
                toast_move: Duration::ZERO,
                group_morph: Duration::ZERO,
                panel_omega: None,
            };
        }
        let speed = config.speed.max(f32::EPSILON);
        Self {
            toast_enter: base.toast_enter.div_f32(speed),
            toast_leave: base.toast_leave.div_f32(speed),
            toast_move: base.toast_move.div_f32(speed),
            group_morph: base.group_morph.div_f32(speed),
            // A spring settles in time proportional to 1/omega.
            panel_omega: base.panel_omega.map(|w| w * speed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Glide {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
}

impl Glide {
    fn at(&self, now: Instant) -> f32 {
        let t = progress(self.start, now, self.dur);
        self.from + (self.to - self.from) * ease_out_cubic(t)
    }
}

/// Fraction of `dur` elapsed since `start` (an empty duration is done).
fn progress(start: Instant, now: Instant, dur: Duration) -> f32 {
    if dur.is_zero() {
        return 1.0;
    }
    (now.saturating_duration_since(start).as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0)
}

/// Where and how to draw one toast this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToastFrame {
    pub id: NotificationId,
    pub y: f32,
    /// Horizontal slide offset (positive = towards the screen edge).
    pub dx: f32,
    pub alpha: f32,
    /// True for a toast that is fading out after removal.
    pub leaving: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Live {
    id: NotificationId,
    born: Instant,
    glide: Glide,
}

#[derive(Debug, Clone, PartialEq)]
struct Leaving {
    id: NotificationId,
    y: f32,
    start: Instant,
}

/// Enter/move/leave motion for the toast stack.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToastMotion {
    live: Vec<Live>,
    leaving: Vec<Leaving>,
    timing: Timing,
}

impl ToastMotion {
    /// An empty stack animating with `timing`.
    #[must_use]
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            ..Self::default()
        }
    }

    /// Sets the stack's target slots (`(id, y)`, newest first). New ids slide
    /// in, moved ids glide, missing ids start leaving. Returns the ids that
    /// started leaving (callers keep their rendering until they finish).
    pub fn sync(&mut self, targets: &[(NotificationId, f32)], now: Instant) -> Vec<NotificationId> {
        let mut gone = Vec::new();
        let old = std::mem::take(&mut self.live);
        for l in &old {
            if !targets.iter().any(|(id, _)| *id == l.id) {
                self.leaving.retain(|x| x.id != l.id);
                self.leaving.push(Leaving {
                    id: l.id,
                    y: l.glide.at(now),
                    start: now,
                });
                gone.push(l.id);
            }
        }
        for &(id, y) in targets {
            // A re-shown id cancels its own leave animation.
            self.leaving.retain(|x| x.id != id);
            let entry = match old.iter().find(|l| l.id == id) {
                Some(l) if (l.glide.to - y).abs() < 0.01 => l.clone(),
                Some(l) => Live {
                    id,
                    born: l.born,
                    glide: Glide {
                        from: l.glide.at(now),
                        to: y,
                        start: now,
                        dur: self.timing.toast_move,
                    },
                },
                None => Live {
                    id,
                    born: now,
                    glide: Glide {
                        from: y,
                        to: y,
                        start: now,
                        dur: self.timing.toast_move,
                    },
                },
            };
            self.live.push(entry);
        }
        gone
    }

    /// Drops finished leave animations; returns their ids.
    pub fn reap(&mut self, now: Instant) -> Vec<NotificationId> {
        let (done, keep): (Vec<_>, Vec<_>) = self
            .leaving
            .drain(..)
            .partition(|l| now.saturating_duration_since(l.start) >= self.timing.toast_leave);
        self.leaving = keep;
        done.into_iter().map(|l| l.id).collect()
    }

    /// Frames to draw at `now`, leaving toasts first (drawn underneath).
    #[must_use]
    pub fn frames(&self, now: Instant) -> Vec<ToastFrame> {
        let leaving = self.leaving.iter().map(|l| {
            let t = ease_out_cubic(progress(l.start, now, self.timing.toast_leave));
            ToastFrame {
                id: l.id,
                y: l.y,
                dx: TOAST_SLIDE * t,
                alpha: 1.0 - t,
                leaving: true,
            }
        });
        let live = self.live.iter().map(|l| {
            let t = ease_out_cubic(progress(l.born, now, self.timing.toast_enter));
            ToastFrame {
                id: l.id,
                y: l.glide.at(now),
                dx: TOAST_SLIDE * (1.0 - t),
                alpha: t,
                leaving: false,
            }
        });
        leaving.chain(live).collect()
    }

    /// Whether anything is still moving at `now`.
    #[must_use]
    pub fn is_animating(&self, now: Instant) -> bool {
        !self.leaving.is_empty()
            || self.live.iter().any(|l| {
                now.saturating_duration_since(l.born) < self.timing.toast_enter
                    || now.saturating_duration_since(l.glide.start) < l.glide.dur
            })
    }
}

/// How long a panel group takes to fan out from its stack (or fold back).
/// A reversal partway through takes proportionally less.
pub const GROUP_MORPH: Duration = Duration::from_millis(320);

/// An eased run of a value from `from` to `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
}

impl Tween {
    fn at(&self, now: Instant) -> f32 {
        let t = progress(self.start, now, self.dur);
        self.from + (self.to - self.from) * ease_out_cubic(t)
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

/// Expand/collapse motion of the panel's groups.
///
/// Fed every group's expanded flag; when one flips, its expansion
/// (0 = stack, 1 = list) eases towards the new state, starting from
/// wherever a running morph had got to.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMotion {
    /// Last known expanded flag per group key.
    expanded: HashMap<String, bool>,
    running: HashMap<String, Tween>,
    /// A full stack-to-list morph.
    duration: Duration,
}

impl Default for GroupMotion {
    fn default() -> Self {
        Self::new(GROUP_MORPH)
    }
}

impl GroupMotion {
    /// No groups yet; a full morph takes `duration`.
    #[must_use]
    pub fn new(duration: Duration) -> Self {
        Self {
            expanded: HashMap::new(),
            running: HashMap::new(),
            duration,
        }
    }

    /// Records the current flags and starts a morph for every group whose
    /// flag changed. New groups appear in place; vanished ones are forgotten.
    pub fn sync<'a>(&mut self, groups: impl IntoIterator<Item = (&'a str, bool)>, now: Instant) {
        let mut seen = HashMap::with_capacity(self.expanded.len());
        for (key, expanded) in groups {
            if let Some(&was) = self.expanded.get(key)
                && was != expanded
            {
                let rest = if was { 1.0 } else { 0.0 };
                let from = self
                    .running
                    .get(key)
                    .filter(|t| !t.done(now))
                    .map_or(rest, |t| t.at(now));
                let to = if expanded { 1.0 } else { 0.0 };
                self.running.insert(
                    key.to_owned(),
                    Tween {
                        from,
                        to,
                        start: now,
                        dur: self.duration.mul_f32((to - from).abs()),
                    },
                );
            }
            seen.insert(key.to_owned(), expanded);
        }
        self.running
            .retain(|key, t| seen.contains_key(key) && !t.done(now));
        self.expanded = seen;
    }

    /// The expansion of every group mid-morph, sorted by key. Groups at rest
    /// are absent.
    #[must_use]
    pub fn frame(&self, now: Instant) -> Vec<(String, f32)> {
        let mut out: Vec<_> = self
            .running
            .iter()
            .filter(|(_, t)| !t.done(now))
            .map(|(key, t)| (key.clone(), t.at(now)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[must_use]
    pub fn is_animating(&self, now: Instant) -> bool {
        self.running.values().any(|t| !t.done(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_follows_the_animation_settings() {
        let fast = Timing::new(&AnimationConfig {
            enabled: true,
            speed: 2.0,
        });
        assert_eq!(fast.group_morph, GROUP_MORPH / 2);
        assert_eq!(fast.toast_enter, TOAST_ENTER / 2);
        assert_eq!(fast.panel_omega, Some(Spring::PANEL_OMEGA * 2.0));
        assert_eq!(Timing::new(&AnimationConfig::default()), Timing::default());
    }

    #[test]
    fn disabled_animation_is_instant() {
        let off = Timing::new(&AnimationConfig {
            enabled: false,
            speed: 1.0,
        });
        let t0 = Instant::now();

        let mut panel = PanelMotion::new(off.panel_omega);
        assert_eq!(panel.animate_to(1.0, t0), Some(true), "settles at once");
        assert!((panel.openness() - 1.0).abs() < f32::EPSILON);

        let mut toasts = ToastMotion::new(off);
        toasts.sync(&[(NotificationId(1), 0.0)], t0);
        assert!(!toasts.is_animating(t0));
        assert!((toasts.frames(t0)[0].alpha - 1.0).abs() < f32::EPSILON);
        toasts.sync(&[], t0);
        assert_eq!(toasts.reap(t0), vec![NotificationId(1)]);

        let mut groups = GroupMotion::new(off.group_morph);
        groups.sync([("g", false)], t0);
        groups.sync([("g", true)], t0);
        assert!(!groups.is_animating(t0));
    }

    #[test]
    fn group_morph_runs_both_ways_and_reverses_midway() {
        let t0 = Instant::now();
        let mut g = GroupMotion::default();
        g.sync([("chat", false), ("mail", false)], t0);
        assert!(!g.is_animating(t0), "first sight is not a change");

        g.sync([("chat", true), ("mail", false)], t0);
        assert!(g.is_animating(t0));
        let mid = g.frame(t0 + ms(80));
        assert_eq!(mid.len(), 1);
        let (key, p) = &mid[0];
        assert_eq!(key, "chat");
        assert!(*p > 0.2 && *p < 1.0, "{p}");

        // Collapsing halfway starts from where it got to, and is shorter.
        g.sync([("chat", false), ("mail", false)], t0 + ms(80));
        let back = g.frame(t0 + ms(81));
        assert!((back[0].1 - p).abs() < 0.05);
        assert!(!g.is_animating(t0 + ms(80) + GROUP_MORPH.mul_f32(*p)));
        assert_eq!(g.frame(t0 + ms(1000)), Vec::new());

        // A vanished group is dropped.
        g.sync([("chat", true)], t0 + ms(2000));
        g.sync(std::iter::empty(), t0 + ms(2001));
        assert!(!g.is_animating(t0 + ms(2001)));
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn easing_endpoints() {
        assert!(ease_out_cubic(0.0).abs() < 1e-6);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-6);
        assert!(ease_out_cubic(0.5) > 0.5);
        assert!((panel_opacity(0.0)).abs() < 1e-6);
        assert!((panel_opacity(1.0) - 1.0).abs() < 1e-6);
        assert!(panel_opacity(0.5) > 0.7);
    }

    #[test]
    fn spring_settles_without_overshoot_from_rest() {
        let mut s = Spring::new(0.0, 0.0, 1.0, Spring::PANEL_OMEGA);
        let mut t = 0.0;
        let mut max: f32 = 0.0;
        while !s.step(1.0 / 60.0) {
            t += 1.0 / 60.0;
            max = max.max(s.value);
            assert!(t < 1.0, "spring must settle within a second");
        }
        assert!(max <= 1.0 + 1e-4);
        assert!((s.value - 1.0).abs() < f32::EPSILON);
        assert!(t > 0.2 && t < 0.5, "settle time {t}");
    }

    #[test]
    fn spring_is_frame_rate_independent() {
        let mut a = Spring::new(0.0, 0.0, 1.0, 20.0);
        let mut b = a;
        a.step(0.1);
        for _ in 0..10 {
            b.step(0.01);
        }
        assert!((a.value - b.value).abs() < 1e-4);
    }

    #[test]
    fn panel_track_jumps_and_animate_settles_once() {
        let t0 = Instant::now();
        let mut p = PanelMotion::default();
        p.track(0.4, t0);
        assert!((p.openness() - 0.4).abs() < 1e-6);
        assert!(!p.is_animating());
        assert_eq!(p.animate_to(1.0, t0 + ms(200)), None);
        let mut settles = Vec::new();
        let mut t = t0 + ms(200);
        for _ in 0..120 {
            t += ms(16);
            let s = p.step(t);
            settles.extend(s.settled);
        }
        assert_eq!(settles, vec![true]);
        assert!((p.openness() - 1.0).abs() < 1e-6);
        assert!(!p.is_animating());
    }

    #[test]
    fn track_interrupts_animation() {
        let t0 = Instant::now();
        let mut p = PanelMotion::default();
        p.animate_to(1.0, t0);
        p.step(t0 + ms(50));
        assert!(p.is_animating());
        p.track(0.2, t0 + ms(60));
        assert!(!p.is_animating());
        assert!((p.openness() - 0.2).abs() < 1e-6);
        // Steps while tracking don't move it or settle.
        assert_eq!(p.step(t0 + ms(100)).settled, None);
        assert!((p.openness() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn release_inherits_finger_velocity() {
        let t0 = Instant::now();
        let mut flick = PanelMotion::default();
        flick.track(0.3, t0);
        flick.track(0.5, t0 + ms(16));
        flick.track(0.7, t0 + ms(32));
        flick.animate_to(1.0, t0 + ms(40));
        let mut still = PanelMotion::default();
        still.track(0.7, t0);
        still.animate_to(1.0, t0 + ms(40));
        let a = flick.step(t0 + ms(56)).openness;
        let b = still.step(t0 + ms(56)).openness;
        assert!(a > b, "a flick carries momentum: {a} vs {b}");
    }

    #[test]
    fn a_finger_that_paused_has_no_momentum() {
        let t0 = Instant::now();
        let mut paused = PanelMotion::default();
        paused.track(0.3, t0);
        paused.track(0.5, t0 + ms(16));
        // Held still for a while before the last sample.
        paused.track(0.7, t0 + ms(300));
        paused.animate_to(1.0, t0 + ms(310));
        let mut still = PanelMotion::default();
        still.track(0.7, t0);
        still.animate_to(1.0, t0 + ms(10));
        let a = paused.step(t0 + ms(326)).openness;
        let b = still.step(t0 + ms(26)).openness;
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    #[test]
    fn animate_to_current_rest_settles_immediately() {
        let mut p = PanelMotion::default();
        assert_eq!(p.animate_to(0.0, Instant::now()), Some(false));
    }

    #[test]
    fn retarget_mid_animation_keeps_velocity() {
        let t0 = Instant::now();
        let mut p = PanelMotion::default();
        p.animate_to(1.0, t0);
        let o1 = p.step(t0 + ms(40)).openness;
        p.animate_to(0.0, t0 + ms(40));
        let o2 = p.step(t0 + ms(48)).openness;
        // Still moving outward briefly thanks to inherited velocity.
        assert!(o2 >= o1 - 0.05);
    }

    #[test]
    fn toasts_enter_glide_and_leave() {
        let t0 = Instant::now();
        let (a, b) = (NotificationId(1), NotificationId(2));
        let mut m = ToastMotion::default();
        m.sync(&[(a, 0.0)], t0);
        let f = m.frames(t0);
        assert_eq!(f.len(), 1);
        assert!(
            f[0].alpha < 0.01 && f[0].dx > 40.0,
            "starts off to the right, transparent"
        );
        let f = m.frames(t0 + ms(400));
        assert!((f[0].alpha - 1.0).abs() < 1e-6 && f[0].dx.abs() < 1e-3);
        assert!(!m.is_animating(t0 + ms(400)));

        // b arrives on top; a glides down.
        let t1 = t0 + ms(400);
        m.sync(&[(b, 0.0), (a, 100.0)], t1);
        let mid = m.frames(t1 + ms(100));
        let ya = mid
            .iter()
            .find(|f| f.id == a)
            .map(|f| f.y)
            .unwrap_or_default();
        assert!(ya > 0.0 && ya < 100.0);
        assert!(m.is_animating(t1 + ms(100)));

        // a is removed: it leaves from where it is.
        let t2 = t1 + ms(500);
        assert_eq!(m.sync(&[(b, 0.0)], t2), vec![a]);
        let f = m.frames(t2 + ms(100));
        let leaving = f.iter().find(|f| f.id == a).copied();
        assert!(leaving.is_some_and(|l| l.leaving && l.alpha < 1.0 && (l.y - 100.0).abs() < 1e-3));
        assert_eq!(m.reap(t2 + ms(100)), Vec::new());
        assert_eq!(m.reap(t2 + ms(300)), vec![a]);
        assert_eq!(m.frames(t2 + ms(300)).len(), 1);
    }

    #[test]
    fn reshown_toast_cancels_leave() {
        let t0 = Instant::now();
        let a = NotificationId(1);
        let mut m = ToastMotion::default();
        m.sync(&[(a, 0.0)], t0);
        m.sync(&[], t0 + ms(10));
        m.sync(&[(a, 0.0)], t0 + ms(20));
        assert_eq!(
            m.frames(t0 + ms(20)).iter().filter(|f| f.id == a).count(),
            1
        );
    }
}
