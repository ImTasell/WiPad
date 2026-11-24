use std::io;
use std::time::{Duration, Instant};

use crate::controller::{ControllerState, InputSequenceTracker};
use crate::protocol::ApplicationMessage;

pub const INPUT_RELEASE_AFTER: Duration = Duration::from_secs(2);
pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionAction {
    Apply(ControllerState),
    Release,
    ControllerConnected,
    ControllerDisconnected,
}

/// Pure per-connection input state. Construct a new tracker for every authenticated
/// transport session so sequence numbers and freshness never leak across reconnects.
pub struct SessionInputTracker {
    sequence: InputSequenceTracker,
    controller_connected: Option<bool>,
    last_input: Instant,
    controls_released: bool,
}

impl SessionInputTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            sequence: InputSequenceTracker::default(),
            controller_connected: None,
            last_input: now,
            controls_released: true,
        }
    }

    pub fn handle(
        &mut self,
        message: ApplicationMessage,
        now: Instant,
    ) -> io::Result<Option<SessionAction>> {
        match message {
            ApplicationMessage::InputState(state) => {
                if !self.sequence.accept(&state)? {
                    return Ok(None);
                }
                self.last_input = now;
                if self.controller_connected == Some(false) {
                    return Ok(None);
                }
                self.controls_released = false;
                Ok(Some(SessionAction::Apply(state)))
            }
            ApplicationMessage::ControllerStatus { connected } => {
                if self.controller_connected == Some(connected) {
                    return Ok(None);
                }
                self.last_input = now;
                self.controller_connected = Some(connected);
                if connected {
                    Ok(Some(SessionAction::ControllerConnected))
                } else {
                    self.controls_released = true;
                    Ok(Some(SessionAction::ControllerDisconnected))
                }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message is not valid input-session state",
            )),
        }
    }

    /// Releases held controls after a short input stall and terminates a dead session.
    pub fn poll(&mut self, now: Instant) -> io::Result<Option<SessionAction>> {
        let idle = now.saturating_duration_since(self.last_input);
        if idle >= SESSION_IDLE_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "controller stream was idle for 15 seconds",
            ));
        }
        if !self.controls_released && idle >= INPUT_RELEASE_AFTER {
            self.controls_released = true;
            return Ok(Some(SessionAction::Release));
        }
        Ok(None)
    }

    pub fn last_sequence(&self) -> Option<u64> {
        self.sequence.last_sequence()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{BUTTON_SOUTH, ControllerState};
    use crate::protocol::{decode_application_message, encode_application_message, write_frame, FrameReader};

    fn pressed(sequence: u64) -> ControllerState {
        ControllerState {
            sequence,
            buttons: BUTTON_SOUTH,
            ..ControllerState::neutral(sequence, sequence * 16_000)
        }
    }

    #[test]
    fn accepts_a_wire_encoded_snapshot_and_applies_it() {
        let start = Instant::now();
        let mut session = SessionInputTracker::new(start);
        let payload =
            encode_application_message(&ApplicationMessage::InputState(pressed(1))).unwrap();
        let mut framed = Vec::new();
        write_frame(&mut framed, &payload).unwrap();
        let mut transport = std::io::Cursor::new(framed);
        let mut decoder = FrameReader::new();
        let received = decoder.read_next(&mut transport).unwrap().unwrap();
        let message = decode_application_message(&received).unwrap();

        assert_eq!(
            session.handle(message, start).unwrap(),
            Some(SessionAction::Apply(pressed(1)))
        );
        assert_eq!(decoder.read_next(&mut transport).unwrap(), None);
        assert_eq!(session.last_sequence(), Some(1));
    }

    #[test]
    fn disconnect_releases_controls_and_ignores_later_snapshots() {
        let start = Instant::now();
        let mut session = SessionInputTracker::new(start);
        assert_eq!(
            session.handle(ApplicationMessage::InputState(pressed(1)), start).unwrap(),
            Some(SessionAction::Apply(pressed(1)))
        );
        assert_eq!(
            session.handle(ApplicationMessage::ControllerStatus { connected: false }, start).unwrap(),
            Some(SessionAction::ControllerDisconnected)
        );
        assert_eq!(
            session.handle(ApplicationMessage::InputState(pressed(2)), start).unwrap(),
            None
        );
        assert_eq!(session.poll(start + Duration::from_secs(3)).unwrap(), None);
    }

    #[test]
    fn reconnecting_controller_requires_a_fresh_snapshot() {
        let start = Instant::now();
        let mut session = SessionInputTracker::new(start);
        session.handle(ApplicationMessage::ControllerStatus { connected: false }, start).unwrap();
        assert_eq!(
            session.handle(ApplicationMessage::ControllerStatus { connected: true }, start).unwrap(),
            Some(SessionAction::ControllerConnected)
        );
        assert_eq!(
            session.handle(ApplicationMessage::InputState(pressed(1)), start).unwrap(),
            Some(SessionAction::Apply(pressed(1)))
        );
    }

    #[test]
    fn stale_or_duplicate_snapshots_do_not_refresh_session_liveness() {
        let start = Instant::now();
        let mut session = SessionInputTracker::new(start);
        session.handle(ApplicationMessage::InputState(pressed(4)), start).unwrap();
        assert_eq!(
            session.handle(
                ApplicationMessage::InputState(pressed(4)),
                start + Duration::from_secs(14),
            ).unwrap(),
            None
        );
        assert_eq!(
            session.poll(start + Duration::from_secs(15)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn input_stall_releases_once_then_times_out() {
        let start = Instant::now();
        let mut session = SessionInputTracker::new(start);
        session.handle(ApplicationMessage::InputState(pressed(1)), start).unwrap();

        assert_eq!(
            session.poll(start + INPUT_RELEASE_AFTER).unwrap(),
            Some(SessionAction::Release)
        );
        assert_eq!(
            session.poll(start + Duration::from_secs(3)).unwrap(),
            None
        );
        assert_eq!(
            session.poll(start + SESSION_IDLE_TIMEOUT).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn a_new_transport_session_starts_with_a_fresh_sequence_window() {
        let start = Instant::now();
        let mut first = SessionInputTracker::new(start);
        first.handle(ApplicationMessage::InputState(pressed(99)), start).unwrap();

        let mut reconnected = SessionInputTracker::new(start + Duration::from_secs(1));
        assert_eq!(
            reconnected
                .handle(
                    ApplicationMessage::InputState(pressed(1)),
                    start + Duration::from_secs(1),
                )
                .unwrap(),
            Some(SessionAction::Apply(pressed(1)))
        );
        assert_eq!(reconnected.last_sequence(), Some(1));
    }

    #[test]
    fn rejects_messages_that_are_not_input_session_messages() {
        let mut session = SessionInputTracker::new(Instant::now());
        assert_eq!(
            session
                .handle(ApplicationMessage::SessionReady, Instant::now())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
