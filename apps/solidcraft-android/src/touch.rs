//! Fingers to pointer: one finger is the left mouse button, two or more are a camera gesture.
//!
//! egui-winit turns the first finger of a touch into the left button the moment it lands, so the
//! second finger of a pinch arrives after the first has already pressed: on the model that starts
//! a selection box or drags whatever was under it. [`TouchRouter`] does the translation instead.
//! It drops egui-winit's pointer events and makes its own from the touches (which it passes on,
//! for egui's multi-touch state), pressing only once a lone finger moves, is held still, or
//! lifts (a tap). A second finger before then makes the touch a gesture, which presses nothing;
//! the camera reads the gesture from egui's multi-touch state (`navigation::handle` in
//! solidcraft-ui-egui).
//!
//! Android delivers a mouse's clicks as touches too (winit 0.30 has no other mouse input there),
//! so every pointer event in a frame comes from a finger or a click, never from anything else.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, TouchId, TouchPhase};

/// How far (points) a lone finger moves before it presses: closer, it is still a tap or the start
/// of a gesture.
pub const SLOP: f32 = 8.0;

/// How long (seconds) a lone finger held still waits before it presses: a hold (Select Other, a
/// drag that starts slowly) needs the button down, and a second finger within it makes a gesture.
pub const HOLD: f64 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// No finger down.
    Idle,
    /// One finger down, nothing pressed yet.
    Pending { id: TouchId, at: Pos2, since: f64 },
    /// One finger down with the left button pressed.
    Pressed { id: TouchId },
    /// Two or more fingers came down: nothing presses until every finger is up.
    Gesture,
}

/// The touch-to-pointer translation, kept from frame to frame.
#[derive(Debug)]
pub struct TouchRouter {
    state: State,
    /// The fingers down, in the order they landed, each where it was last.
    fingers: Vec<(TouchId, Pos2)>,
    /// A tap's release, sent at the start of the next frame so the press is seen on its own first.
    release: Option<Pos2>,
}

impl Default for TouchRouter {
    fn default() -> Self {
        Self { state: State::Idle, fingers: Vec::new(), release: None }
    }
}

impl TouchRouter {
    /// Rewrite this frame's input. Returns how long (seconds) until the router needs another frame
    /// even without new input: a finger held still is about to press, or a tap's release is due.
    pub fn route(&mut self, raw: &mut RawInput) -> Option<f64> {
        let now = raw.time.unwrap_or(0.0);
        let modifiers = raw.modifiers;
        let mut out = Vec::with_capacity(raw.events.len() + 4);
        if let Some(pos) = self.release.take() {
            out.push(button(pos, false, modifiers));
            out.push(Event::PointerGone);
        }
        for event in std::mem::take(&mut raw.events) {
            match event {
                // egui-winit's translation of the same touches: this router's replaces it.
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone => {}
                Event::Touch { id, phase, pos, .. } => {
                    out.push(event);
                    self.touch(id, phase, pos, now, modifiers, &mut out);
                }
                other => out.push(other),
            }
        }
        if let State::Pending { id, at, since } = self.state
            && now - since >= HOLD
        {
            press(at, modifiers, &mut out);
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

    fn touch(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, now: f64, modifiers: Modifiers, out: &mut Vec<Event>) {
        if !(pos.x.is_finite() && pos.y.is_finite()) {
            return;
        }
        match phase {
            TouchPhase::Start => {
                self.fingers.retain(|(f, _)| *f != id);
                self.fingers.push((id, pos));
                self.state = match self.state {
                    State::Idle if self.fingers.len() == 1 => {
                        out.push(Event::PointerMoved(pos));
                        State::Pending { id, at: pos, since: now }
                    }
                    State::Pressed { id: first } => {
                        // Too late for a gesture that presses nothing: let go where the first
                        // finger is, and the rest of the touch is the gesture.
                        let at = self.finger(first).unwrap_or(pos);
                        out.push(button(at, false, modifiers));
                        State::Gesture
                    }
                    _ => State::Gesture,
                };
                if self.state == State::Gesture {
                    self.point_at_centre(out);
                }
            }
            TouchPhase::Move => {
                let Some(finger) = self.fingers.iter_mut().find(|(f, _)| *f == id) else { return };
                finger.1 = pos;
                match self.state {
                    State::Pending { id: first, at, .. } if first == id => {
                        if pos.distance(at) > SLOP {
                            // A drag: it starts where the finger landed, not where it is now.
                            press(at, modifiers, out);
                            self.state = State::Pressed { id };
                        }
                        out.push(Event::PointerMoved(pos));
                    }
                    State::Pressed { id: first } if first == id => out.push(Event::PointerMoved(pos)),
                    State::Gesture => self.point_at_centre(out),
                    _ => {}
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                // A lifted finger says where it left; a cancelled one may not mean it.
                let at = if phase == TouchPhase::End { pos } else { self.finger(id).unwrap_or(pos) };
                self.fingers.retain(|(f, _)| *f != id);
                match self.state {
                    State::Pending { id: first, at: down, .. } if first == id => {
                        if phase == TouchPhase::End {
                            // A tap: pressed now, let go on the next frame.
                            press(down, modifiers, out);
                            self.release = Some(down);
                        } else {
                            out.push(Event::PointerGone);
                        }
                        self.state = State::Idle;
                    }
                    State::Pressed { id: first } if first == id => {
                        out.push(Event::PointerMoved(at));
                        out.push(button(at, false, modifiers));
                        out.push(Event::PointerGone);
                        self.state = State::Idle;
                    }
                    State::Gesture if !self.fingers.is_empty() => self.point_at_centre(out),
                    _ => {}
                }
                // Every finger is up: nothing stays pressed, whatever was missed on the way.
                if self.fingers.is_empty() {
                    match self.state {
                        State::Pressed { .. } => {
                            out.push(button(at, false, modifiers));
                            out.push(Event::PointerGone);
                        }
                        State::Pending { .. } | State::Gesture => out.push(Event::PointerGone),
                        State::Idle => {}
                    }
                    self.state = State::Idle;
                }
            }
        }
    }

    fn finger(&self, id: TouchId) -> Option<Pos2> {
        self.fingers.iter().find(|(f, _)| *f == id).map(|(_, p)| *p)
    }

    /// The pointer follows the middle of the fingers during a gesture: a pinch zooms about it.
    fn point_at_centre(&self, out: &mut Vec<Event>) {
        if self.fingers.is_empty() {
            return;
        }
        let n = self.fingers.len() as f32;
        let sum = self.fingers.iter().fold(egui::Vec2::ZERO, |s, (_, p)| s + p.to_vec2());
        out.push(Event::PointerMoved((sum / n).to_pos2()));
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn press(at: Pos2, modifiers: Modifiers, out: &mut Vec<Event>) {
    out.push(Event::PointerMoved(at));
    out.push(button(at, true, modifiers));
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

    /// The left button's presses (true) and releases (false), in order.
    fn buttons(events: &[Event]) -> Vec<(bool, Pos2)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { pressed, pos, button: PointerButton::Primary, .. } => Some((*pressed, *pos)),
                _ => None,
            })
            .collect()
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
        assert!(all.iter().any(|e| matches!(e, Event::PointerMoved(p) if *p == Pos2::new(50.0, 50.0))), "the pointer follows the middle");
        assert!(matches!(all.last(), Some(Event::PointerGone)));
        assert!(!r.active());
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
