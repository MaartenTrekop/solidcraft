//! Fingers to pointer: one finger is the left mouse button, two or more are a camera gesture, two
//! fingers tapped together are a right click, and three fingers swiped left or right are Undo or
//! Redo.
//!
//! egui-winit turns the first finger of a touch into the left button the moment it lands, so the
//! second finger of a pinch arrives after the first has already pressed: on the model that starts
//! a selection box or drags whatever was under it. [`TouchRouter`] does the translation instead.
//! It drops egui-winit's pointer events and makes its own from the touches (which it passes on,
//! for egui's multi-touch state), pressing only once a lone finger moves, is held still, or
//! lifts (a tap). A second finger before then makes the touch a gesture, which presses nothing;
//! the camera reads the gesture from egui's multi-touch state (`navigation::handle` in
//! solidcraft-ui-egui). Two fingers that lift soon without moving were a tap: a right click
//! where the first one landed, which opens the context menus the desktop opens on a right click.
//!
//! Android delivers a mouse's clicks as touches too (winit 0.30 has no other mouse input there),
//! so every pointer event in a frame comes from a finger or a click, never from anything else.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, TouchId, TouchPhase};

/// How far (points) a finger moves before it counts as moving: closer, a lone finger is still a
/// tap or the start of a gesture, and two fingers still a tap.
pub const SLOP: f32 = 8.0;

/// How long (seconds) a lone finger held still waits before it presses: a hold (Select Other, a
/// drag that starts slowly) needs the button down, and a second finger within it makes a gesture.
pub const HOLD: f64 = 0.15;

/// The longest (seconds) two fingers may stay down and still be a tap (a right click).
pub const TWO_FINGER_TAP: f64 = 0.4;

/// How far (points) three fingers move sideways, and at least twice as far as up or down, to be
/// a swipe: to the left Undo, to the right Redo, once per touch.
pub const SWIPE: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// No finger down.
    Idle,
    /// One finger down, nothing pressed yet.
    Pending { id: TouchId, at: Pos2, since: f64 },
    /// One finger down with the left button pressed.
    Pressed { id: TouchId },
    /// Two or more fingers came down: nothing presses until every finger is up. `tap`: where the
    /// first finger landed and when, while the touch can still be a two-finger tap.
    Gesture { tap: Option<(Pos2, f64)> },
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: TouchId,
    /// Where it is now.
    at: Pos2,
    /// Where it landed.
    from: Pos2,
}

/// The touch-to-pointer translation, kept from frame to frame.
#[derive(Debug)]
pub struct TouchRouter {
    state: State,
    /// The fingers down, in the order they landed.
    fingers: Vec<Finger>,
    /// A tap's release, sent at the start of the next frame so the press is seen on its own first.
    release: Option<(Pos2, PointerButton)>,
    /// The keyboard's modifiers as the input last said (a Bluetooth keyboard's Shift or Ctrl).
    modifiers: Modifiers,
    /// Three or more fingers down: where their middle was when the third landed, and whether
    /// this touch has already swiped.
    swipe: Option<(Pos2, bool)>,
}

impl Default for TouchRouter {
    fn default() -> Self {
        Self { state: State::Idle, fingers: Vec::new(), release: None, modifiers: Modifiers::NONE, swipe: None }
    }
}

impl TouchRouter {
    /// Rewrite this frame's input. Returns how long (seconds) until the router needs another frame
    /// even without new input: a finger held still is about to press, or a tap's release is due.
    pub fn route(&mut self, raw: &mut RawInput) -> Option<f64> {
        let now = raw.time.unwrap_or(0.0);
        let mut out = Vec::with_capacity(raw.events.len() + 4);
        if let Some((pos, which)) = self.release.take() {
            out.push(button(pos, which, false, self.modifiers));
            out.push(Event::PointerGone);
        }
        for event in std::mem::take(&mut raw.events) {
            match event {
                // egui-winit's translation of the same touches: this router's replaces it.
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone => {}
                Event::Touch { id, phase, pos, .. } => {
                    out.push(event);
                    self.touch(id, phase, pos, now, &mut out);
                }
                Event::ModifiersChanged(modifiers) => {
                    self.modifiers = modifiers;
                    out.push(event);
                }
                other => out.push(other),
            }
        }
        if let State::Pending { id, at, since } = self.state
            && now - since >= HOLD
        {
            press(at, self.modifiers, &mut out);
            self.state = State::Pressed { id };
        }
        raw.events = out;
        match self.state {
            State::Pending { since, .. } => Some((since + HOLD - now).max(0.0)),
            _ if self.release.is_some() => Some(0.0),
            _ => None,
        }
    }

    /// Whether a finger is down (or a tap's release is still to come).
    pub fn active(&self) -> bool {
        self.state != State::Idle || self.release.is_some()
    }

    fn touch(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, now: f64, out: &mut Vec<Event>) {
        if !(pos.x.is_finite() && pos.y.is_finite()) {
            return;
        }
        let modifiers = self.modifiers;
        match phase {
            TouchPhase::Start => {
                self.fingers.retain(|f| f.id != id);
                self.fingers.push(Finger { id, at: pos, from: pos });
                self.state = match self.state {
                    State::Idle if self.fingers.len() == 1 => {
                        out.push(Event::PointerMoved(pos));
                        State::Pending { id, at: pos, since: now }
                    }
                    State::Pending { at, since, .. } if self.fingers.len() == 2 => State::Gesture { tap: Some((at, since)) },
                    State::Pressed { id: first } => {
                        // Too late for a gesture that presses nothing: let go where the first
                        // finger is, and the rest of the touch is the gesture.
                        let at = self.finger(first).map_or(pos, |f| f.at);
                        out.push(button(at, PointerButton::Primary, false, modifiers));
                        State::Gesture { tap: None }
                    }
                    _ => State::Gesture { tap: None },
                };
                if matches!(self.state, State::Gesture { .. }) {
                    self.point_at_centre(out);
                }
                if self.fingers.len() >= 3 {
                    let swiped = self.swipe.is_some_and(|(_, swiped)| swiped);
                    self.swipe = self.centre().map(|c| (c, swiped));
                }
            }
            TouchPhase::Move => {
                let Some(finger) = self.fingers.iter_mut().find(|f| f.id == id) else { return };
                finger.at = pos;
                let moved = pos.distance(finger.from) > SLOP;
                match self.state {
                    State::Pending { id: first, at, .. } if first == id => {
                        if moved {
                            // A drag: it starts where the finger landed, not where it is now.
                            press(at, modifiers, out);
                            self.state = State::Pressed { id };
                        }
                        out.push(Event::PointerMoved(pos));
                    }
                    State::Pressed { id: first } if first == id => out.push(Event::PointerMoved(pos)),
                    State::Gesture { tap } => {
                        if moved && tap.is_some() {
                            self.state = State::Gesture { tap: None };
                        }
                        self.point_at_centre(out);
                        if self.fingers.len() >= 3
                            && let Some((from, false)) = self.swipe
                            && let Some(now) = self.centre()
                        {
                            let d = now - from;
                            if d.x.abs() >= SWIPE && d.x.abs() >= 2.0 * d.y.abs() {
                                undo_or_redo(d.x > 0.0, out);
                                self.swipe = Some((from, true));
                            }
                        }
                    }
                    _ => {}
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                // A lifted finger says where it left; a cancelled one may not mean it.
                let at = if phase == TouchPhase::End { pos } else { self.finger(id).map_or(pos, |f| f.at) };
                self.fingers.retain(|f| f.id != id);
                if self.fingers.len() < 3 {
                    // Kept as swiped until every finger is up: one swipe per touch.
                    self.swipe = self.swipe.filter(|(_, swiped)| *swiped && !self.fingers.is_empty());
                }
                match self.state {
                    State::Pending { id: first, at: down, .. } if first == id => {
                        if phase == TouchPhase::End {
                            // A tap: pressed now, let go on the next frame.
                            press(down, modifiers, out);
                            self.release = Some((down, PointerButton::Primary));
                        } else {
                            out.push(Event::PointerGone);
                        }
                        self.state = State::Idle;
                    }
                    State::Pressed { id: first } if first == id => {
                        out.push(Event::PointerMoved(at));
                        out.push(button(at, PointerButton::Primary, false, modifiers));
                        out.push(Event::PointerGone);
                        self.state = State::Idle;
                    }
                    State::Gesture { .. } if !self.fingers.is_empty() => self.point_at_centre(out),
                    _ => {}
                }
                // Every finger is up: nothing stays pressed, whatever was missed on the way.
                if self.fingers.is_empty() {
                    match self.state {
                        State::Pressed { .. } => {
                            out.push(button(at, PointerButton::Primary, false, modifiers));
                            out.push(Event::PointerGone);
                        }
                        State::Gesture { tap: Some((down, since)) } if phase == TouchPhase::End && now - since <= TWO_FINGER_TAP => {
                            // Two fingers tapped together: a right click where the first landed,
                            // let go on the next frame.
                            out.push(Event::PointerMoved(down));
                            out.push(button(down, PointerButton::Secondary, true, modifiers));
                            self.release = Some((down, PointerButton::Secondary));
                        }
                        State::Pending { .. } | State::Gesture { .. } => out.push(Event::PointerGone),
                        State::Idle => {}
                    }
                    self.state = State::Idle;
                }
            }
        }
    }

    fn finger(&self, id: TouchId) -> Option<Finger> {
        self.fingers.iter().find(|f| f.id == id).copied()
    }

    /// The pointer follows the middle of the fingers during a gesture: a pinch zooms about it.
    fn point_at_centre(&self, out: &mut Vec<Event>) {
        if let Some(centre) = self.centre() {
            out.push(Event::PointerMoved(centre));
        }
    }

    /// The middle of the fingers down.
    fn centre(&self) -> Option<Pos2> {
        if self.fingers.is_empty() {
            return None;
        }
        let n = self.fingers.len() as f32;
        let sum = self.fingers.iter().fold(egui::Vec2::ZERO, |s, f| s + f.at.to_vec2());
        Some((sum / n).to_pos2())
    }
}

/// Undo (Ctrl+Z) or Redo (Ctrl+Y) as the keyboard asks for them, so they go where the keys go:
/// a spline's last point first, nothing while a text field has the keyboard.
fn undo_or_redo(redo: bool, out: &mut Vec<Event>) {
    let key = if redo { Key::Y } else { Key::Z };
    let modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for pressed in [true, false] {
        out.push(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
    }
}

fn button(pos: Pos2, which: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: which, pressed, modifiers }
}

fn press(at: Pos2, modifiers: Modifiers, out: &mut Vec<Event>) {
    out.push(Event::PointerMoved(at));
    out.push(button(at, PointerButton::Primary, true, modifiers));
}

/// Android's Back button (and gesture) as Escape: it cancels a tool or closes a dialog, as Esc does
/// on the desktop, instead of reaching the app as a key nothing handles.
pub fn back_is_escape(raw: &mut RawInput) {
    for event in &mut raw.events {
        if let Event::Key { key, physical_key, .. } = event
            && *key == Key::BrowserBack
        {
            *key = Key::Escape;
            *physical_key = Some(Key::Escape);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, x: f32, y: f32) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: TouchId(id), phase, pos: Pos2::new(x, y), force: None }
    }

    /// One frame through the router: the pointer events it made (touches left out).
    fn frame(router: &mut TouchRouter, time: f64, events: Vec<Event>) -> Vec<Event> {
        let mut raw = RawInput { time: Some(time), events, ..RawInput::default() };
        router.route(&mut raw);
        raw.events.into_iter().filter(|e| !matches!(e, Event::Touch { .. })).collect()
    }

    /// The presses (true) and releases (false) of `which`, in order, with where they were.
    fn presses(events: &[Event], which: PointerButton) -> Vec<(bool, Pos2)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { pressed, pos, button, .. } if *button == which => Some((*pressed, *pos)),
                _ => None,
            })
            .collect()
    }

    fn buttons(events: &[Event]) -> Vec<(bool, Pos2)> {
        presses(events, PointerButton::Primary)
    }

    #[test]
    fn a_tap_presses_and_lets_go_on_the_next_frame() {
        let mut r = TouchRouter::default();
        assert!(buttons(&frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 10.0, 20.0)])).is_empty());
        let lift = frame(&mut r, 0.05, vec![touch(1, TouchPhase::End, 11.0, 20.0)]);
        assert_eq!(buttons(&lift), vec![(true, Pos2::new(10.0, 20.0))]);
        assert!(r.active(), "the release is still to come");
        let next = frame(&mut r, 0.06, Vec::new());
        assert_eq!(buttons(&next), vec![(false, Pos2::new(10.0, 20.0))]);
        assert!(matches!(next.last(), Some(Event::PointerGone)));
        assert!(!r.active());
    }

    #[test]
    fn a_lone_finger_that_moves_drags_from_where_it_landed() {
        let mut r = TouchRouter::default();
        frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 10.0, 10.0)]);
        assert!(buttons(&frame(&mut r, 0.02, vec![touch(1, TouchPhase::Move, 12.0, 10.0)])).is_empty(), "within the slop");
        let moved = frame(&mut r, 0.04, vec![touch(1, TouchPhase::Move, 40.0, 10.0)]);
        assert_eq!(buttons(&moved), vec![(true, Pos2::new(10.0, 10.0))]);
        assert!(matches!(moved.last(), Some(Event::PointerMoved(p)) if *p == Pos2::new(40.0, 10.0)));
        let up = frame(&mut r, 0.08, vec![touch(1, TouchPhase::End, 50.0, 10.0)]);
        assert_eq!(buttons(&up), vec![(false, Pos2::new(50.0, 10.0))], "let go where the finger left");
    }

    #[test]
    fn a_finger_held_still_presses() {
        let mut r = TouchRouter::default();
        frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 5.0, 5.0)]);
        let mut raw = RawInput { time: Some(0.05), ..RawInput::default() };
        let wait = r.route(&mut raw);
        assert!(wait.is_some_and(|w| (w - (HOLD - 0.05)).abs() < 1e-9), "asks for the frame that presses");
        let held = frame(&mut r, HOLD + 0.01, Vec::new());
        assert_eq!(buttons(&held), vec![(true, Pos2::new(5.0, 5.0))]);
    }

    #[test]
    fn a_second_finger_soon_after_the_first_presses_nothing() {
        let mut r = TouchRouter::default();
        let mut all = frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 0.0, 0.0)]);
        all.extend(frame(&mut r, 0.03, vec![touch(2, TouchPhase::Start, 100.0, 0.0)]));
        all.extend(frame(&mut r, 0.5, vec![touch(1, TouchPhase::Move, 0.0, 50.0), touch(2, TouchPhase::Move, 100.0, 50.0)]));
        all.extend(frame(&mut r, 0.6, vec![touch(1, TouchPhase::End, 0.0, 50.0)]));
        all.extend(frame(&mut r, 0.7, vec![touch(2, TouchPhase::End, 100.0, 50.0)]));
        assert!(buttons(&all).is_empty(), "{all:?}");
        assert!(presses(&all, PointerButton::Secondary).is_empty(), "a gesture that moved is no tap");
        assert!(all.iter().any(|e| matches!(e, Event::PointerMoved(p) if *p == Pos2::new(50.0, 50.0))), "the pointer follows the middle");
        assert!(matches!(all.last(), Some(Event::PointerGone)));
        assert!(!r.active());
    }

    #[test]
    fn two_fingers_tapped_together_right_click_where_the_first_landed() {
        let mut r = TouchRouter::default();
        frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 20.0, 30.0)]);
        frame(&mut r, 0.02, vec![touch(2, TouchPhase::Start, 60.0, 30.0)]);
        let lift = frame(&mut r, 0.15, vec![touch(1, TouchPhase::End, 21.0, 30.0), touch(2, TouchPhase::End, 61.0, 31.0)]);
        assert_eq!(presses(&lift, PointerButton::Secondary), vec![(true, Pos2::new(20.0, 30.0))]);
        assert!(buttons(&lift).is_empty());
        let next = frame(&mut r, 0.16, Vec::new());
        assert_eq!(presses(&next, PointerButton::Secondary), vec![(false, Pos2::new(20.0, 30.0))]);
        assert!(!r.active());
    }

    #[test]
    fn two_fingers_held_too_long_are_no_right_click() {
        let mut r = TouchRouter::default();
        frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 20.0, 30.0)]);
        frame(&mut r, 0.02, vec![touch(2, TouchPhase::Start, 60.0, 30.0)]);
        let lift = frame(&mut r, 1.0, vec![touch(1, TouchPhase::End, 20.0, 30.0), touch(2, TouchPhase::End, 60.0, 30.0)]);
        assert!(presses(&lift, PointerButton::Secondary).is_empty());
    }

    /// The keys a frame pressed, with Ctrl or not.
    fn keys(events: &[Event]) -> Vec<(Key, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } => Some((*key, modifiers.ctrl)),
                _ => None,
            })
            .collect()
    }

    fn three_fingers(r: &mut TouchRouter, dx: f32, dy: f32) -> Vec<Event> {
        let mut all = frame(r, 0.0, vec![touch(1, TouchPhase::Start, 100.0, 100.0)]);
        all.extend(frame(r, 0.01, vec![touch(2, TouchPhase::Start, 140.0, 100.0), touch(3, TouchPhase::Start, 180.0, 100.0)]));
        for step in 1..=4 {
            let f = step as f32 / 4.0;
            let moves = (1..=3u64).map(|id| touch(id, TouchPhase::Move, 60.0 + 40.0 * id as f32 + dx * f, 100.0 + dy * f)).collect();
            all.extend(frame(r, 0.02 + 0.02 * f64::from(step), moves));
        }
        all.extend(frame(r, 0.2, (1..=3u64).map(|id| touch(id, TouchPhase::End, 60.0 + 40.0 * id as f32 + dx, 100.0 + dy)).collect()));
        all
    }

    #[test]
    fn three_fingers_swiped_left_undo_once() {
        let mut r = TouchRouter::default();
        let all = three_fingers(&mut r, -150.0, 10.0);
        assert_eq!(keys(&all), vec![(Key::Z, true)]);
        assert!(buttons(&all).is_empty());
        assert!(presses(&all, PointerButton::Secondary).is_empty());
    }

    #[test]
    fn three_fingers_swiped_right_redo() {
        let mut r = TouchRouter::default();
        assert_eq!(keys(&three_fingers(&mut r, 150.0, -5.0)), vec![(Key::Y, true)]);
    }

    #[test]
    fn three_fingers_moved_up_or_down_or_not_far_do_nothing() {
        let mut r = TouchRouter::default();
        assert!(keys(&three_fingers(&mut r, 40.0, 150.0)).is_empty());
        assert!(keys(&three_fingers(&mut r, -30.0, 0.0)).is_empty());
    }

    #[test]
    fn a_second_finger_after_a_press_lets_go_first() {
        let mut r = TouchRouter::default();
        frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, 0.0, 0.0)]);
        frame(&mut r, 0.02, vec![touch(1, TouchPhase::Move, 30.0, 0.0)]);
        let second = frame(&mut r, 0.3, vec![touch(2, TouchPhase::Start, 100.0, 0.0)]);
        assert_eq!(buttons(&second), vec![(false, Pos2::new(30.0, 0.0))]);
        let rest = frame(&mut r, 0.4, vec![touch(1, TouchPhase::End, 30.0, 0.0), touch(2, TouchPhase::End, 100.0, 0.0)]);
        assert!(buttons(&rest).is_empty());
        assert!(presses(&rest, PointerButton::Secondary).is_empty());
    }

    #[test]
    fn egui_winit_pointer_events_are_replaced() {
        let mut r = TouchRouter::default();
        let events = vec![
            Event::PointerMoved(Pos2::new(1.0, 1.0)),
            Event::PointerButton { pos: Pos2::new(1.0, 1.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            touch(7, TouchPhase::Start, 1.0, 1.0),
            Event::Text("a".into()),
        ];
        let out = frame(&mut r, 0.0, events);
        assert!(buttons(&out).is_empty());
        assert!(out.iter().any(|e| matches!(e, Event::Text(t) if t == "a")), "other input passes through");
    }

    #[test]
    fn presses_carry_the_keyboard_modifiers() {
        let mut r = TouchRouter::default();
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        frame(&mut r, 0.0, vec![Event::ModifiersChanged(shift), touch(1, TouchPhase::Start, 0.0, 0.0)]);
        let lift = frame(&mut r, 0.05, vec![touch(1, TouchPhase::End, 0.0, 0.0)]);
        assert!(lift.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, modifiers, .. } if modifiers.shift)));
    }

    #[test]
    fn hostile_positions_are_ignored() {
        let mut r = TouchRouter::default();
        let out = frame(&mut r, 0.0, vec![touch(1, TouchPhase::Start, f32::NAN, 0.0)]);
        assert!(out.is_empty());
        assert!(!r.active());
    }

    #[test]
    fn back_is_escape_rewrites_only_back() {
        let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let mut raw = RawInput { events: vec![key(Key::BrowserBack), key(Key::A)], ..RawInput::default() };
        back_is_escape(&mut raw);
        assert!(matches!(raw.events.first(), Some(Event::Key { key: Key::Escape, .. })));
        assert!(matches!(raw.events.get(1), Some(Event::Key { key: Key::A, .. })));
    }
}
