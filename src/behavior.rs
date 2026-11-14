//! Pure behavior layer for V6: support placement, pointer reactions and
//! elapsed-time animation selection. Never changes semantic status.
//! All timers are `Duration` budgets advanced by `tick(dt)` only while
//! playing, so pause/hide freezes them instead of resetting.
//!
//! Geometry is compositor-logical margin space: the usable screen
//! rectangle is measured on real output edges, the window clamp region
//! (`allowed`) is the separately-extended top-left bounds, and frame
//! dimensions are the *displayed* surface size.

use std::time::Duration;

use crate::hyprland;

/// Support pose the character is placed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Standing,
    Floor,
    WallLeft,
    WallRight,
}

impl Placement {
    /// Animation-name prefix for V2 assets.
    pub fn prefix(self) -> &'static str {
        match self {
            Placement::Standing => "standing",
            Placement::Floor => "floor",
            Placement::WallLeft => "wall_left",
            Placement::WallRight => "wall_right",
        }
    }

    /// Single-token name for --status / CLI.
    pub fn token(self) -> &'static str {
        match self {
            Placement::Standing => "standing",
            Placement::Floor => "floor",
            Placement::WallLeft => "wall_left",
            Placement::WallRight => "wall_right",
        }
    }

    /// Parse a --pose value.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "standing" => Placement::Standing,
            "floor" => Placement::Floor,
            "wall-left" | "wall_left" => Placement::WallLeft,
            "wall-right" | "wall_right" => Placement::WallRight,
            _ => return None,
        })
    }

    /// Contact point within the frame, as a fraction of frame size
    /// (source pixels 256: standing/floor bottom-center, walls at edges).
    pub fn contact_frac(self) -> (f64, f64) {
        match self {
            Placement::Standing | Placement::Floor => (0.5, 248.0 / 256.0),
            Placement::WallLeft => (8.0 / 256.0, 248.0 / 256.0),
            Placement::WallRight => (248.0 / 256.0, 248.0 / 256.0),
        }
    }
}

/// Pointer interaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    Blush,
    Dragged,
}

impl Reaction {
    pub fn token(self) -> &'static str {
        match self {
            Reaction::Blush => "blush",
            Reaction::Dragged => "dragged",
        }
    }
}

/// Usable SCREEN rectangle in compositor logical coordinates: x0,y0 is
/// the usable origin and x1,y1 are the real right/bottom usable edges.
/// This is NOT the window top-left clamp region.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// Per-family canonical contact points (normalized 0..1 of the frame),
/// read from each family's `_idle` manifest entry. Defaults match the
/// documented V6 contract contacts.
#[derive(Debug, Clone, Copy)]
pub struct SupportContacts {
    pub standing: (f64, f64),
    pub floor: (f64, f64),
    pub wall_left: (f64, f64),
    pub wall_right: (f64, f64),
}

impl SupportContacts {
    pub fn get(self, p: Placement) -> (f64, f64) {
        match p {
            Placement::Standing => self.standing,
            Placement::Floor => self.floor,
            Placement::WallLeft => self.wall_left,
            Placement::WallRight => self.wall_right,
        }
    }
}

impl Default for SupportContacts {
    fn default() -> Self {
        Self {
            standing: Placement::Standing.contact_frac(),
            floor: Placement::Floor.contact_frac(),
            wall_left: Placement::WallLeft.contact_frac(),
            wall_right: Placement::WallRight.contact_frac(),
        }
    }
}

/// All geometry a support/snap decision needs: real screen edges, the
/// allowed window top-left clamp (transparent inset already applied),
/// the displayed frame size and per-family contacts.
#[derive(Debug, Clone, Copy)]
pub struct PlacementGeometry {
    pub screen: Bounds,
    pub allowed: hyprland::Bounds,
    pub frame: (i32, i32),
    pub contacts: SupportContacts,
}

impl PlacementGeometry {
    /// Clamp a window top-left position into the allowed region.
    pub fn clamp(self, pos: (i32, i32)) -> (i32, i32) {
        (
            pos.0.clamp(self.allowed.min_x, self.allowed.max_x),
            pos.1.clamp(self.allowed.min_y, self.allowed.max_y),
        )
    }

    /// World position of placement `p`'s contact when the window sits at
    /// `pos` (window top-left + contact offset in displayed px).
    pub fn contact(self, pos: (i32, i32), p: Placement) -> (i32, i32) {
        let c = self.contacts.get(p);
        (
            pos.0 + (c.0 * self.frame.0 as f64).round() as i32,
            pos.1 + (c.1 * self.frame.1 as f64).round() as i32,
        )
    }

    /// Window position seating placement `p`'s contact GUTTER px inside
    /// its support edge, preserving the release position parallel to the
    /// edge (floor keeps drop X, walls keep drop Y). No edge recentering.
    pub fn snap(self, pos: (i32, i32), p: Placement) -> (i32, i32) {
        let c = self.contact((0, 0), p);
        self.clamp(match p {
            Placement::Standing => pos,
            Placement::Floor => (pos.0, self.screen.y1 - GUTTER - c.1),
            Placement::WallLeft => (self.screen.x0 + GUTTER - c.0, pos.1),
            Placement::WallRight => (self.screen.x1 - GUTTER - c.0, pos.1),
        })
    }
}

/// Production geometry builder used by both the desktop tick and tests:
/// screen = the real usable output rect, allowed = window clamp extended
/// by the transparent inset on left/right/bottom only.
pub fn placement_geometry(
    mon: &hyprland::MonitorInfo,
    frame: (i32, i32),
    inset: i32,
    contacts: SupportContacts,
) -> PlacementGeometry {
    let (uw, uh) = hyprland::usable_size(mon);
    let mut allowed = hyprland::usable_bounds(mon, frame.0, frame.1);
    allowed.min_x -= inset;
    allowed.max_x += inset;
    allowed.max_y += inset;
    PlacementGeometry {
        screen: Bounds {
            x0: 0,
            y0: 0,
            x1: uw,
            y1: uh,
        },
        allowed,
        frame,
        contacts,
    }
}

const CLICK_THRESHOLD: i32 = 8;
const CAPTURE_BAND: i32 = 32;
const DETACH_DIST: i32 = 56;
const GUTTER: i32 = 2;
pub const BLUSH: Duration = Duration::from_millis(1800);
pub const CLICK_COOLDOWN: Duration = Duration::from_millis(1200);
/// Chain window for consecutive clicks: a third sub-threshold click
/// inside it promotes to a chat-open request instead of another blush.
pub const TRIPLE_CLICK: Duration = Duration::from_millis(700);
pub const SETTLE: Duration = Duration::from_millis(350);
pub const JOY_MAX: Duration = Duration::from_secs(6);
pub const FOCUS_ALTERNATE: Duration = Duration::from_secs(12);
pub const AMBIENT_CYCLE: Duration = Duration::from_secs(30);
const AMBIENT_DANCE_START: Duration = Duration::from_secs(18);
const AMBIENT_DANCE_END: Duration = Duration::from_secs(24);

#[derive(Debug)]
struct Press {
    start_x: i32,
    start_y: i32,
    dragging: bool,
}

/// Behavior controller: placement, reactions, timer budgets.
#[derive(Debug)]
pub struct Behavior {
    pub placement: Placement,
    pub reaction: Option<Reaction>,
    press: Option<Press>,
    blush_left: Duration,
    cooldown_left: Duration,
    settle_left: Duration,
    /// CLI --pose override: display-only, snaps through the same path.
    pub forced_placement: Option<Placement>,
    /// Established support: placement + its post-snap WORLD contact.
    anchored: Option<(Placement, i32, i32)>,
    /// Consecutive sub-threshold clicks within TRIPLE_CLICK of each other.
    click_streak: u8,
    click_chain_left: Duration,
    open_chat: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self::new(Placement::Standing)
    }
}

impl Behavior {
    pub fn new(placement: Placement) -> Self {
        Self {
            placement,
            reaction: None,
            press: None,
            blush_left: Duration::ZERO,
            cooldown_left: Duration::ZERO,
            settle_left: Duration::ZERO,
            forced_placement: None,
            anchored: None,
            click_streak: 0,
            click_chain_left: Duration::ZERO,
            open_chat: false,
        }
    }

    /// Primary press. `pos` = current window position (logical).
    pub fn press(&mut self, pos: (i32, i32)) {
        self.press = Some(Press {
            start_x: pos.0,
            start_y: pos.1,
            dragging: false,
        });
        self.settle_left = Duration::ZERO;
    }

    /// Pointer moved while pressed. >=8px Euclidean travel (including
    /// exactly 8px and diagonals like 6,6) becomes a drag. Returns the
    /// candidate window position once dragging.
    pub fn update(&mut self, pos: (i32, i32)) -> Option<(i32, i32)> {
        let p = self.press.as_mut()?;
        let dx = pos.0 as i64 - p.start_x as i64;
        let dy = pos.1 as i64 - p.start_y as i64;
        if !p.dragging && (dx as f64).hypot(dy as f64) >= CLICK_THRESHOLD as f64 {
            p.dragging = true;
            self.reaction = Some(Reaction::Dragged);
            self.blush_left = Duration::ZERO;
        }
        p.dragging.then_some(pos)
    }

    /// Release: requires a captured press (a cancel followed by a GTK
    /// drag-end can never become a phantom click). A drag-end whose final
    /// position crossed the threshold counts even without prior updates.
    pub fn release(&mut self, pos: (i32, i32), geometry: PlacementGeometry) -> Option<(i32, i32)> {
        let press = self.press.take()?;
        let dx = pos.0 as i64 - press.start_x as i64;
        let dy = pos.1 as i64 - press.start_y as i64;
        let dragged = press.dragging || (dx as f64).hypot(dy as f64) >= CLICK_THRESHOLD as f64;
        if dragged {
            self.reaction = Some(Reaction::Dragged);
            self.settle_left = SETTLE;
            return self.snap_at(pos, geometry);
        }
        // Click below threshold: streak counts consecutive clicks chained
        // by TRIPLE_CLICK; the third promotes to a chat-open request.
        self.click_streak = if self.click_chain_left.is_zero() {
            1
        } else {
            self.click_streak + 1
        };
        self.click_chain_left = TRIPLE_CLICK;
        if self.click_streak >= 3 {
            self.click_streak = 0;
            self.open_chat = true;
        }
        // Click below threshold: blush, subject to cooldown.
        if self.cooldown_left.is_zero() {
            self.reaction = Some(Reaction::Blush);
            self.blush_left = BLUSH;
            self.cooldown_left = CLICK_COOLDOWN;
        }
        None
    }

    /// True once after the third chained click; consumed by the caller.
    pub fn take_open_chat(&mut self) -> bool {
        std::mem::take(&mut self.open_chat)
    }

    /// Cancel capture only (input path gone, e.g. click-through): no
    /// phantom click, visual reactions keep their own clocks. A captured
    /// drag never got a settle timer, so its Dragged reaction is dropped
    /// here instead of lingering forever; settled/other reactions stay.
    pub fn cancel_capture(&mut self) {
        let was_dragging = self.press.take().is_some_and(|p| p.dragging);
        if was_dragging && self.reaction == Some(Reaction::Dragged) {
            self.reaction = None;
            self.settle_left = Duration::ZERO;
        }
    }

    /// Cancel interaction (pause/hide/fullscreen/popover/monitor loss,
    /// GTK gesture cancel): drops capture and a captured drag pose.
    pub fn cancel(&mut self) {
        self.press = None;
        // A cancelled drag must not leave the pet stuck in `dragged`.
        if self.reaction == Some(Reaction::Dragged) {
            self.reaction = None;
        }
    }

    /// Full cancel for hide/fullscreen/pause/monitor loss: clears
    /// capture, transient reactions and timers but KEEPS the established
    /// support anchor and placement so a later resize can re-anchor.
    pub fn cancel_all(&mut self) {
        self.cancel();
        self.reaction = None;
        self.blush_left = Duration::ZERO;
        self.cooldown_left = Duration::ZERO;
        self.settle_left = Duration::ZERO;
    }

    /// Clear forced/transient reactions (demo switch, --live) while
    /// preserving the support anchor, placement and click cooldown.
    pub fn clear_transients(&mut self) {
        self.reaction = None;
        self.blush_left = Duration::ZERO;
        self.settle_left = Duration::ZERO;
    }

    /// Contact-aware snap on drag release. Each candidate is measured
    /// with its OWN target contact against the real screen edges: floor
    /// always wins when in band (corners included), then the nearer
    /// qualifying wall, otherwise free standing. No top snap.
    fn snap_at(&mut self, pos: (i32, i32), geometry: PlacementGeometry) -> Option<(i32, i32)> {
        let pos = geometry.clamp(pos);
        // Degenerate usable rect: never snap, drop support honestly.
        if geometry.screen.x1 - geometry.screen.x0 < 64
            || geometry.screen.y1 - geometry.screen.y0 < 64
        {
            self.placement = Placement::Standing;
            self.anchored = None;
            return Some(pos);
        }
        let fc = geometry.contact(pos, Placement::Floor);
        let lc = geometry.contact(pos, Placement::WallLeft);
        let rc = geometry.contact(pos, Placement::WallRight);
        let d_floor = (geometry.screen.y1 - GUTTER - fc.1).abs();
        let d_left = (lc.0 - (geometry.screen.x0 + GUTTER)).abs();
        let d_right = (rc.0 - (geometry.screen.x1 - GUTTER)).abs();
        let p = if d_floor <= CAPTURE_BAND {
            Placement::Floor
        } else {
            match (d_left <= CAPTURE_BAND, d_right <= CAPTURE_BAND) {
                (true, true) => {
                    if d_left <= d_right {
                        Placement::WallLeft
                    } else {
                        Placement::WallRight
                    }
                }
                (true, false) => Placement::WallLeft,
                (false, true) => Placement::WallRight,
                (false, false) => {
                    self.placement = Placement::Standing;
                    self.anchored = None;
                    return Some(pos);
                }
            }
        };
        self.placement = p;
        let snapped = geometry.snap(pos, p);
        let (wx, wy) = geometry.contact(snapped, p);
        self.anchored = Some((p, wx, wy));
        Some(snapped)
    }

    /// While a drag is held over an anchored support, normal-to-support
    /// displacement >56px detaches (floor: vertical, walls: horizontal).
    /// Parallel movement never detaches. Uses the anchored family's
    /// contact even while the `dragged` reaction is showing.
    pub fn held_drag_update(&mut self, pos: (i32, i32), geometry: PlacementGeometry) {
        if !self.press.as_ref().map(|p| p.dragging).unwrap_or(false) {
            return;
        }
        if let Some((p, ax, ay)) = self.anchored {
            let (cx, cy) = geometry.contact(pos, p);
            let normal_disp = match p {
                Placement::Standing | Placement::Floor => (cy - ay).abs(),
                Placement::WallLeft | Placement::WallRight => (cx - ax).abs(),
            };
            if normal_disp > DETACH_DIST {
                self.anchored = None;
                self.placement = Placement::Standing;
            }
        }
    }

    /// Re-anchor after resize/unplug: keep the stored support on its edge
    /// and preserve (then clamp) the parallel coordinate. The world
    /// contact stored in the anchor is refreshed to the post-clamp seat.
    pub fn reanchor(&mut self, pos: (i32, i32), geometry: PlacementGeometry) -> Option<(i32, i32)> {
        let (p, ax, ay) = self.anchored?;
        let c = geometry.contact((0, 0), p);
        let target = match p {
            // Normal coordinate reseats on the new edge; parallel keeps
            // the stored world contact position.
            Placement::Floor => (ax - c.0, geometry.screen.y1 - GUTTER - c.1),
            Placement::WallLeft => (geometry.screen.x0 + GUTTER - c.0, ay - c.1),
            Placement::WallRight => (geometry.screen.x1 - GUTTER - c.0, ay - c.1),
            Placement::Standing => pos,
        };
        let snapped = geometry.clamp(target);
        let (wx, wy) = geometry.contact(snapped, p);
        self.anchored = Some((p, wx, wy));
        Some(snapped)
    }

    /// Advance timer budgets. Call only with the visible-playback delta.
    /// Returns true when the reaction ended this tick.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let mut changed = false;
        if !self.blush_left.is_zero() {
            self.blush_left = self.blush_left.saturating_sub(dt);
            if self.blush_left.is_zero() && self.reaction == Some(Reaction::Blush) {
                self.reaction = None;
                changed = true;
            }
        }
        let was_settling = !self.settle_left.is_zero();
        self.settle_left = self.settle_left.saturating_sub(dt);
        // Dragged pose ends with the 350ms settle after release.
        if was_settling && self.settle_left.is_zero() && self.reaction == Some(Reaction::Dragged) {
            self.reaction = None;
            changed = true;
        }
        self.cooldown_left = self.cooldown_left.saturating_sub(dt);
        let was_chained = !self.click_chain_left.is_zero();
        self.click_chain_left = self.click_chain_left.saturating_sub(dt);
        if was_chained && self.click_chain_left.is_zero() {
            self.click_streak = 0;
        }
        changed
    }

    /// True while a pointer press is captured.
    pub fn is_pressing(&self) -> bool {
        self.press.is_some()
    }

    /// CLI --pose: display-only placement override snapped through the
    /// same contact/edge path as a real drag release. `standing` clears
    /// support and keeps the clamped position — it is NOT a floor snap.
    pub fn apply_pose(
        &mut self,
        p: Placement,
        pos: (i32, i32),
        geometry: PlacementGeometry,
    ) -> Option<(i32, i32)> {
        // Degenerate/missing screen: never synthesize a support.
        // Same guard as snap_at — clear support, stand, clamp only.
        if geometry.screen.x1 - geometry.screen.x0 < 64
            || geometry.screen.y1 - geometry.screen.y0 < 64
        {
            self.forced_placement = None;
            self.placement = Placement::Standing;
            self.anchored = None;
            return Some(geometry.clamp(pos));
        }
        self.forced_placement = Some(p);
        self.placement = p;
        if p == Placement::Standing {
            self.anchored = None;
            return Some(geometry.clamp(pos));
        }
        let snapped = geometry.snap(pos, p);
        let (wx, wy) = geometry.contact(snapped, p);
        self.anchored = Some((p, wx, wy));
        Some(snapped)
    }

    /// A new awaiting-input/error episode ends an active blush early.
    /// A drag reaction and the click cooldown are left untouched.
    pub fn interrupt_blush(&mut self) {
        if self.reaction == Some(Reaction::Blush) {
            self.reaction = None;
            self.blush_left = Duration::ZERO;
        }
    }

    /// Clear the --pose override; the manually/support-snapped placement
    /// itself is kept (--live semantics).
    pub fn clear_pose(&mut self) {
        self.forced_placement = None;
    }

    /// Display-only reaction override for --demo blush/dragged.
    pub fn force_reaction(&mut self, r: Reaction) {
        self.reaction = Some(r);
        match r {
            Reaction::Blush => self.blush_left = BLUSH,
            Reaction::Dragged => {}
        }
    }

    /// True while a blush reaction is visually active.
    pub fn blush_active(&self) -> bool {
        self.reaction == Some(Reaction::Blush) && !self.blush_left.is_zero()
    }

    /// True during the short post-release settle ease.
    pub fn settling(&self) -> bool {
        !self.settle_left.is_zero()
    }

    /// True while a support anchor is established (floor/walls).
    pub fn supported(&self) -> bool {
        self.anchored.is_some()
    }

    /// Ambient idle phase from accumulated *playing* idle time.
    /// Cycle: 0-18s idle, 18-24s dance, 24-30s idle. On Floor the final
    /// segment alternates with floor_recline every other cycle.
    pub fn ambient_phase(&self, idle_elapsed: Duration, on_floor: bool) -> AmbientPhase {
        let t = idle_elapsed.as_millis() as u64 % AMBIENT_CYCLE.as_millis() as u64;
        if t >= AMBIENT_DANCE_START.as_millis() as u64 && t < AMBIENT_DANCE_END.as_millis() as u64 {
            AmbientPhase::Dance
        } else if t >= AMBIENT_DANCE_END.as_millis() as u64 && on_floor {
            let cycle = idle_elapsed.as_millis() as u64 / AMBIENT_CYCLE.as_millis() as u64;
            if cycle % 2 == 1 {
                AmbientPhase::Recline
            } else {
                AmbientPhase::Idle
            }
        } else {
            AmbientPhase::Idle
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientPhase {
    Idle,
    Dance,
    Recline,
}

/// Advance the fresh-completion celebration budget. A NEW completion
/// episode resets it; time only accrues while the effective emotion is
/// Joy and playback is visible. A consumed budget is preserved (frozen)
/// across temporary higher-priority emotions — never reset by them.
pub fn joy_elapsed_next(
    elapsed: Duration,
    new_episode: bool,
    joy: bool,
    playing: bool,
    dt: Duration,
) -> Duration {
    let mut e = if new_episode { Duration::ZERO } else { elapsed };
    if joy && playing {
        e += dt;
    }
    e.min(JOY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion as mot;

    /// The lead's grounded fixture: 1920x1080 output, reserved top 34 ->
    /// usable screen 1920x1046. Displayed frame and inset per scale.
    fn monitor(w: i32, h: i32, top: i32) -> hyprland::MonitorInfo {
        hyprland::MonitorInfo {
            id: 0,
            name: "TEST".into(),
            logical_w: w,
            logical_h: h,
            reserved: [0, top, 0, 0],
            focused: false,
            active_workspace: 1,
            special_workspace: None,
        }
    }

    fn geom(frame: i32, inset: i32) -> PlacementGeometry {
        placement_geometry(
            &monitor(1920, 1080, 34),
            (frame, frame),
            inset,
            SupportContacts::default(),
        )
    }

    /// Contract geometry: scale2 => 320px frame, 8px inset.
    fn g2() -> PlacementGeometry {
        geom(320, 8)
    }

    /// Contract geometry: scale1 => 160px frame, 3px inset.
    fn g1() -> PlacementGeometry {
        geom(160, 3)
    }

    /// Drives a press->release without crossing the drag threshold: a click.
    fn click(b: &mut Behavior, at: (i32, i32)) {
        b.press(at);
        assert!(b.release(at, g2()).is_none(), "a click must not snap");
    }

    /// Drives a real press->drag->release and returns the ACTUAL snapped
    /// window position produced by release (never the drop input).
    fn drag_release(
        b: &mut Behavior,
        from: (i32, i32),
        to: (i32, i32),
        geometry: PlacementGeometry,
    ) -> (i32, i32) {
        b.press(from);
        assert!(b.update(to).is_some(), "move must cross the drag threshold");
        b.release(to, geometry)
            .expect("captured drag must return its position")
    }

    #[test]
    fn triple_click_requests_open_chat() {
        let mut b = Behavior::new(Placement::Standing);
        click(&mut b, (100, 100));
        click(&mut b, (100, 100));
        assert!(!b.take_open_chat(), "two clicks must not open a chat");
        click(&mut b, (100, 100));
        assert!(b.take_open_chat(), "third chained click opens a chat");
        assert!(!b.take_open_chat(), "the request is consumed once");
    }

    #[test]
    fn click_streak_expires_after_window() {
        let mut b = Behavior::new(Placement::Standing);
        click(&mut b, (100, 100));
        click(&mut b, (100, 100));
        b.tick(TRIPLE_CLICK + Duration::from_millis(1));
        click(&mut b, (100, 100));
        assert!(
            !b.take_open_chat(),
            "a click after the chain window restarts the streak"
        );
    }

    #[test]
    fn contract_geometry_1920x1080_scale2() {
        let g = g2();
        // Usable screen is the REAL rect, not clamp+window size.
        assert_eq!(
            (g.screen.x0, g.screen.y0, g.screen.x1, g.screen.y1),
            (0, 0, 1920, 1046)
        );
        // Allowed window clamp extended only by the 8px transparent inset.
        assert_eq!(
            (
                g.allowed.min_x,
                g.allowed.max_x,
                g.allowed.min_y,
                g.allowed.max_y
            ),
            (-8, 1608, 0, 734)
        );
        // Contacts at displayed offsets 10/310 for walls, 160/310 center.
        assert_eq!(g.contact((0, 0), Placement::WallLeft), (10, 310));
        assert_eq!(g.contact((0, 0), Placement::WallRight), (310, 310));
        assert_eq!(g.contact((0, 0), Placement::Floor), (160, 310));
    }

    #[test]
    fn contract_geometry_1920x1080_scale1() {
        let g = g1();
        assert_eq!((g.screen.x1, g.screen.y1), (1920, 1046));
        assert_eq!(
            (
                g.allowed.min_x,
                g.allowed.max_x,
                g.allowed.min_y,
                g.allowed.max_y
            ),
            (-3, 1763, 0, 889)
        );
    }

    #[test]
    fn floor_release_preserves_parallel_x() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        // Near-edge drop (800,718): returned position is the ACTUAL seat
        // (800,734), off-center X preserved.
        let r = drag_release(&mut b, (500, 400), (800, 718), g);
        assert_eq!(r, (800, 734));
        assert_eq!(b.placement, Placement::Floor);
        // Snap returns exactly the seated position: contact y = 1046-2=1044.
        assert_eq!(g.contact(r, Placement::Floor).1, 1044);
    }

    #[test]
    fn wall_release_reachable_and_preserves_y() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        // Near-edge drop (4,300): allowed leftmost x=-8 puts the WALL
        // contact at 2 (0+2 gutter); returned pos is the actual snap.
        let r = drag_release(&mut b, (400, 300), (4, 300), g);
        assert_eq!(r, (-8, 300));
        assert_eq!(b.placement, Placement::WallLeft);
        assert_eq!(g.contact(r, Placement::WallLeft).0, 2);
        let mut b2 = Behavior::new(Placement::Standing);
        let r2 = drag_release(&mut b2, (400, 300), (1595, 300), g);
        assert_eq!(r2, (1608, 300));
        assert_eq!(b2.placement, Placement::WallRight);
        assert_eq!(g.contact(r2, Placement::WallRight).0, 1918);
    }

    #[test]
    fn floor_wins_both_bottom_corners() {
        let g = g2();
        // Near-corner drops: the wall contact can be exactly on the
        // gutter (d=0) yet floor still wins when its band qualifies.
        for (drop, want) in [((-8, 718), (-8, 734)), ((1608, 718), (1608, 734))] {
            let mut b = Behavior::new(Placement::Standing);
            let r = drag_release(&mut b, (400, 300), drop, g);
            assert_eq!(r, want, "drop={drop:?}");
            assert_eq!(b.placement, Placement::Floor, "drop={drop:?}");
        }
    }

    #[test]
    fn snap_survives_next_clamp_tick() {
        let g = g2();
        for (drop, want, placement) in [
            ((800, 718), (800, 734), Placement::Floor),
            ((4, 300), (-8, 300), Placement::WallLeft),
            ((1595, 300), (1608, 300), Placement::WallRight),
        ] {
            let mut b = Behavior::new(Placement::Standing);
            // Feed the ACTUAL returned snap through the stationary tick.
            let pos = drag_release(&mut b, (400, 300), drop, g);
            assert_eq!(pos, want);
            assert_eq!(b.placement, placement);
            // The snapped window position is inside allowed -> a later
            // clamp (motion advance with dt=0) leaves it untouched.
            // Mirror the real release path: drag_to marks the pet pinned
            // so advance does not gravity-drop it back to the floor.
            let mut m = mot::Motion::new(g.allowed);
            m.drag_to(pos.0 as f64, pos.1 as f64, g.allowed);
            m.advance(0.0, g.allowed, false);
            assert_eq!((m.x as i32, m.y as i32), pos, "{placement:?}");
        }
    }

    #[test]
    fn scale1_wall_and_floor_edges() {
        let g = g1();
        let mut b = Behavior::new(Placement::Standing);
        let r = drag_release(&mut b, (300, 400), (400, 880), g);
        assert_eq!(r, (400, 889));
        assert_eq!(b.placement, Placement::Floor);
        assert_eq!(g.contact(r, Placement::Floor).1, 1044);
        let mut b2 = Behavior::new(Placement::Standing);
        let r2 = drag_release(&mut b2, (300, 400), (2, 400), g);
        assert_eq!(r2, (-3, 400));
        assert_eq!(b2.placement, Placement::WallLeft);
        assert_eq!(g.contact(r2, Placement::WallLeft).0, 2);
        let mut b3 = Behavior::new(Placement::Standing);
        let r3 = drag_release(&mut b3, (300, 400), (1750, 400), g);
        assert_eq!(r3, (1763, 400));
        assert_eq!(b3.placement, Placement::WallRight);
        assert_eq!(g.contact(r3, Placement::WallRight).0, 1918);
    }

    #[test]
    fn parallel_keeps_support_normal_detaches() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (-8, 300), g);
        assert_eq!(b.placement, Placement::WallLeft);
        // Parallel (y) movement along the wall: support retained.
        b.press((-8, 300));
        b.update((-8, 600));
        b.held_drag_update((-8, 600), g);
        assert_eq!(b.placement, Placement::WallLeft);
        b.cancel();
        // Normal (x) displacement >56 detaches to standing.
        b.press((-8, 300));
        b.update((60, 300));
        b.held_drag_update((60, 300), g);
        assert_eq!(b.placement, Placement::Standing);
    }

    #[test]
    fn detach_uses_anchored_family_contact() {
        // Floor anchor: 57px vertical move detaches even while `dragged`
        // art is showing (its contact is not consulted).
        let g = g2();
        let mut b = Behavior::new(Placement::Floor);
        let snapped = b.apply_pose(Placement::Floor, (500, 700), g).unwrap();
        assert_eq!(snapped, (500, 734));
        b.press(snapped);
        b.update((500, 800));
        assert_eq!(b.reaction, Some(Reaction::Dragged));
        b.held_drag_update((500, 800), g); // contact y moved 66 > 56
        assert_eq!(b.placement, Placement::Standing);
    }

    #[test]
    fn reanchor_preserves_parallel_coordinate() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (800, 734), g);
        assert_eq!(b.placement, Placement::Floor);
        // Smaller screen: floor edge reseats at new y, contact x kept.
        let small = placement_geometry(
            &monitor(1280, 720, 34),
            (320, 320),
            8,
            SupportContacts::default(),
        );
        let p = b.reanchor((800, 734), small).unwrap();
        // usable 1280x686: floor y = 686-2-310 = 374; x = 960-160 = 800
        // clamped into allowed [ -8, 1280-320+8=968 ].
        assert_eq!(p, (800, 374));
        // Wall resize keeps parallel y.
        let mut w = Behavior::new(Placement::Standing);
        drag_release(&mut w, (400, 300), (-8, 300), g);
        let p2 = w.reanchor((-8, 300), small).unwrap();
        assert_eq!(p2, (-8, 300));
    }

    #[test]
    fn reanchor_uses_new_monitor_rect_on_enlargement() {
        // The stale-list bug read the OLD monitor: an enlarged output
        // must re-anchor against ITS rectangle (usable 2560x1406).
        let g = g2();
        let big = placement_geometry(
            &monitor(2560, 1440, 34),
            (320, 320),
            8,
            SupportContacts::default(),
        );
        assert_eq!((big.screen.x1, big.screen.y1), (2560, 1406));
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (800, 734), g);
        assert_eq!(b.placement, Placement::Floor);
        // Floor reseats at the new bottom edge, parallel x kept.
        assert_eq!(b.reanchor((800, 734), big).unwrap(), (800, 1094));
        let mut w = Behavior::new(Placement::Standing);
        drag_release(&mut w, (400, 300), (1608, 300), g);
        assert_eq!(w.placement, Placement::WallRight);
        // Right wall reseats at the new right edge, parallel y kept.
        assert_eq!(w.reanchor((1608, 300), big).unwrap(), (2248, 300));
    }

    #[test]
    fn reanchor_clamps_parallel_when_too_far() {
        let g = g2();
        let mut w = Behavior::new(Placement::Standing);
        drag_release(&mut w, (400, 300), (-8, 600), g);
        assert_eq!(w.placement, Placement::WallLeft);
        // Tiny vertical space: parallel y clamps into allowed, support kept.
        let short = placement_geometry(
            &monitor(1920, 500, 34),
            (320, 320),
            8,
            SupportContacts::default(),
        );
        let p = w.reanchor((-8, 600), short).unwrap();
        // usable 1920x466 -> allowed y [0, 466-320+8=154]; contact y =
        // 600+310=910 stored -> wy = 910-310=600 -> clamp 154.
        assert_eq!(p, (-8, 154));
        assert!(w.supported());
    }

    #[test]
    fn tiny_usable_rect_never_snaps() {
        // 50x40 usable is never inflated to frame size; release keeps a
        // clamped free-standing position, no support.
        let tiny = placement_geometry(
            &monitor(50, 74, 34),
            (320, 320),
            8,
            SupportContacts::default(),
        );
        let mut b = Behavior::new(Placement::Floor);
        b.press((0, 0));
        b.update((10, 10));
        let r = b.release((10, 10), tiny);
        assert_eq!(b.placement, Placement::Standing);
        assert!(!b.supported());
        // Usable is 50x40: max stays (0,0)+inset -> (8,8), never inflated
        // to the 320px frame. Release clamps into that tiny window.
        assert_eq!(r, Some((8, 8)));
        // --pose on a degenerate/missing screen synthesizes NO support:
        // forced placement and anchor cleared, Standing, clamped pos.
        let r = b.apply_pose(Placement::Floor, (10, 10), tiny).unwrap();
        assert_eq!(r, (8, 8));
        assert_eq!(b.placement, Placement::Standing);
        assert_eq!(b.forced_placement, None);
        assert!(!b.supported());
    }

    #[test]
    fn click_below_threshold_blushes() {
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        assert!(b.update((503, 402)).is_none()); // <8px: still a click
        let r = b.release((503, 402), g2());
        assert!(r.is_none());
        assert_eq!(b.reaction, Some(Reaction::Blush));
        assert!(b.blush_active());
    }

    #[test]
    fn exact_8px_and_diagonal_are_drags() {
        // Exactly 8 horizontal: >= threshold -> drag, no blush.
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        assert!(b.update((508, 400)).is_some());
        b.release((508, 400), g2());
        assert_eq!(b.reaction, Some(Reaction::Dragged));
        assert!(!b.blush_active());
        // Diagonal (6,6): Euclidean 8.49 >= 8 -> drag.
        let mut b2 = Behavior::new(Placement::Standing);
        b2.press((500, 400));
        assert!(b2.update((506, 406)).is_some());
        b2.release((506, 406), g2());
        assert_eq!(b2.reaction, Some(Reaction::Dragged));
        // (5,5) = 7.07 stays a click.
        let mut b3 = Behavior::new(Placement::Standing);
        b3.press((500, 400));
        assert!(b3.update((505, 405)).is_none());
        b3.release((505, 405), g2());
        assert_eq!(b3.reaction, Some(Reaction::Blush));
    }

    #[test]
    fn drag_end_without_prior_update_is_still_drag() {
        // GTK may deliver only begin+end: final position decides.
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        b.release((520, 400), g2());
        assert_eq!(b.reaction, Some(Reaction::Dragged));
    }

    #[test]
    fn cancel_then_release_has_no_phantom_click() {
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        b.cancel();
        // No captured press: release is a no-op, not a phantom click.
        let r = b.release((500, 400), g2());
        assert!(r.is_none());
        assert_eq!(b.reaction, None);
    }

    #[test]
    fn cancel_has_no_phantom_click() {
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        b.update((530, 400));
        b.cancel();
        assert_eq!(b.reaction, None);
        assert!(b.press.is_none());
    }

    #[test]
    fn cancel_capture_drops_captured_drag_keeps_other_clocks() {
        let g = g2();
        // Real captured drag mid-flight: Dragged has NO settle timer yet
        // (set only on release). Click-through cancels the capture —
        // the reaction must not linger forever.
        let mut b = Behavior::new(Placement::Standing);
        b.press((400, 300));
        b.update((600, 300));
        assert_eq!(b.reaction, Some(Reaction::Dragged));
        assert!(b.settle_left.is_zero(), "captured drag has no settle yet");
        b.cancel_capture();
        assert!(b.press.is_none());
        assert_eq!(b.reaction, None, "captured Dragged must not linger");
        // Long tick then a stray release: no press, no phantom blush.
        b.tick(Duration::from_secs(60));
        assert!(b.release((600, 300), g).is_none());
        assert_eq!(b.reaction, None);
    }

    #[test]
    fn cancel_capture_retains_blush_and_forced_demo() {
        // Blush survives a capture cancel and expires on its own clock.
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 400));
        b.release((503, 402), g2()); // click: blush
        assert_eq!(b.reaction, Some(Reaction::Blush));
        b.press((500, 400)); // new press captured, no drag
        b.cancel_capture();
        assert_eq!(b.reaction, Some(Reaction::Blush), "blush survives");
        b.tick(BLUSH + Duration::from_millis(1));
        assert_eq!(b.reaction, None, "blush still expires normally");
        // A forced demo reaction is not a captured drag: survives.
        let mut d = Behavior::new(Placement::Standing);
        d.force_reaction(Reaction::Dragged);
        d.press((10, 10)); // press without crossing the threshold
        d.cancel_capture();
        assert_eq!(d.reaction, Some(Reaction::Dragged), "forced demo kept");
    }

    #[test]
    fn clear_transients_preserves_support() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (800, 734), g);
        assert_eq!(b.placement, Placement::Floor);
        b.force_reaction(Reaction::Dragged);
        b.clear_transients();
        assert_eq!(b.reaction, None);
        assert_eq!(b.placement, Placement::Floor);
        assert!(b.supported());
    }

    #[test]
    fn demo_dragged_exits_via_clear_transients() {
        // --demo dragged -> --live and -> --demo joy both must not leave
        // the pet stuck in the dragged pose.
        let mut b = Behavior::new(Placement::Standing);
        b.force_reaction(Reaction::Dragged);
        assert_eq!(b.reaction, Some(Reaction::Dragged));
        b.clear_transients(); // --live path
        assert_eq!(b.reaction, None);
        b.force_reaction(Reaction::Dragged);
        b.clear_transients(); // demo switch path, then new demo applies
        b.force_reaction(Reaction::Blush);
        assert_eq!(b.reaction, Some(Reaction::Blush));
    }

    #[test]
    fn cancel_all_keeps_support_anchor() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (800, 734), g);
        assert!(b.supported());
        b.press((800, 700));
        b.update((820, 700));
        b.cancel_all();
        // Capture and transients cleared, support survives fullscreen.
        assert!(b.press.is_none());
        assert_eq!(b.reaction, None);
        assert!(b.supported());
        assert_eq!(b.placement, Placement::Floor);
        // A resize after the hidden period can still re-anchor.
        let small = placement_geometry(
            &monitor(1280, 720, 34),
            (320, 320),
            8,
            SupportContacts::default(),
        );
        assert!(b.reanchor((800, 734), small).is_some());
    }

    #[test]
    fn apply_pose_standing_clears_without_floor_snap() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        drag_release(&mut b, (400, 300), (800, 734), g);
        assert_eq!(b.placement, Placement::Floor);
        // --pose standing: support cleared, clamped position kept, NO
        // teleport onto the floor edge.
        let r = b.apply_pose(Placement::Standing, (600, 500), g).unwrap();
        assert_eq!(r, (600, 500));
        assert_eq!(b.placement, Placement::Standing);
        assert!(!b.supported());
        // Supported poses do use the real snap path.
        let r2 = b.apply_pose(Placement::WallRight, (600, 500), g).unwrap();
        assert_eq!(r2, (1608, 500));
        assert_eq!(b.placement, Placement::WallRight);
        assert_eq!(g.contact(r2, Placement::WallRight).0, 1918);
    }

    #[test]
    fn blush_cooldown_and_expiry() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        b.press((0, 0));
        b.release((0, 0), g);
        assert!(b.blush_active());
        // Second click during cooldown: no reaction change.
        b.press((0, 0));
        b.release((0, 0), g);
        assert_eq!(b.reaction, Some(Reaction::Blush));
        // Advance past blush+cooldown.
        b.tick(Duration::from_millis(2000));
        assert!(!b.blush_active());
        assert_eq!(b.reaction, None);
        // Now a new click blushes again.
        b.press((0, 0));
        b.release((0, 0), g);
        assert!(b.blush_active());
    }

    #[test]
    fn ambient_cycle_and_floor_recline() {
        let b = Behavior::new(Placement::Floor);
        assert_eq!(
            b.ambient_phase(Duration::from_secs(5), true),
            AmbientPhase::Idle
        );
        assert_eq!(
            b.ambient_phase(Duration::from_secs(20), true),
            AmbientPhase::Dance
        );
        assert_eq!(
            b.ambient_phase(Duration::from_secs(26), true),
            AmbientPhase::Idle
        );
        assert_eq!(
            b.ambient_phase(Duration::from_secs(56), true),
            AmbientPhase::Recline
        );
        let s = Behavior::new(Placement::Standing);
        assert_eq!(
            s.ambient_phase(Duration::from_secs(56), false),
            AmbientPhase::Idle
        );
    }

    #[test]
    fn joy_budget_episode_and_interruption() {
        // Second fresh completion while already Joy: fresh budget.
        let e = joy_elapsed_next(
            Duration::from_secs(3),
            true,
            true,
            true,
            Duration::from_millis(60),
        );
        assert_eq!(e, Duration::from_millis(60));
        // Interruption (Focus) without a new episode: budget preserved.
        let e = joy_elapsed_next(
            Duration::from_secs(3),
            false,
            false,
            true,
            Duration::from_secs(1),
        );
        assert_eq!(e, Duration::from_secs(3));
        // Returning to Joy consumes the remaining budget, not a new one.
        let e = joy_elapsed_next(e, false, true, true, Duration::from_secs(4));
        assert_eq!(e, JOY_MAX);
        // Frozen while paused even when Joy is showing.
        let e = joy_elapsed_next(
            Duration::from_secs(2),
            false,
            true,
            false,
            Duration::from_secs(1),
        );
        assert_eq!(e, Duration::from_secs(2));
    }

    #[test]
    fn attention_episode_interrupts_blush_not_drag() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        b.press((0, 0));
        b.release((0, 0), g);
        assert!(b.blush_active());
        b.interrupt_blush();
        assert!(!b.blush_active());
        assert_eq!(b.reaction, None);
        // An active drag reaction survives the same interrupt.
        b.press((500, 400));
        b.update((530, 400));
        b.release((800, 400), g);
        b.tick(Duration::from_millis(100)); // inside the 350ms settle
        assert_eq!(b.reaction, Some(Reaction::Dragged));
        b.interrupt_blush();
        assert_eq!(b.reaction, Some(Reaction::Dragged));
    }

    #[test]
    fn no_top_snap() {
        let g = g2();
        let mut b = Behavior::new(Placement::Standing);
        b.press((500, 10));
        b.update((510, 10));
        b.release((510, 10), g);
        // Top of the screen is never a support: free standing.
        assert_eq!(b.placement, Placement::Standing);
    }
}
