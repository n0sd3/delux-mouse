//! Device service: tracks what the mouse reports and guards every write.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use delux_hid::{Device, HidError};
use delux_profile::{Profile, Section};
use delux_proto::{
    encode_buttons, encode_factory_reset, encode_polling, encode_power_light, encode_sensor,
    frames, BatteryState, Event, EventKind, Frame, MODEL_M900PRO_8K_MS4,
};
use serde::Serialize;

/// No event for longer than this means the mouse is asleep (or off) and
/// would silently miss a command.
pub const AWAKE_WINDOW_MS: u64 = 6000;

/// The transport, as the service needs it.
pub trait Link: Send {
    fn read_event(&self, timeout_ms: i32) -> Result<Option<Event>, HidError>;
    fn write_frames(&self, frames: &[Frame]) -> Result<(), HidError>;
}

impl Link for Device {
    fn read_event(&self, timeout_ms: i32) -> Result<Option<Event>, HidError> {
        Device::read_event(self, timeout_ms)
    }
    fn write_frames(&self, frames: &[Frame]) -> Result<(), HidError> {
        Device::write_frames(self, frames)
    }
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Monotonic wall clock.
pub struct SystemClock(std::time::Instant);

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Connection {
    NoReceiver,
    Asleep,
    Awake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub connection: Connection,
    pub model: Option<u8>,
    /// Last battery reading; see `battery_is_stale`.
    pub battery: Option<(BatteryState, u8)>,
    /// True when `battery` is a remembered value, not a current one.
    pub battery_is_stale: bool,
    pub dpi_stage: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("receptor não conectado")]
    NoReceiver,
    #[error("mouse dormindo ou desligado — mexa o mouse e tente de novo")]
    Asleep,
    #[error("modelo {0:#04x} não suportado para escrita")]
    UnsupportedModel(u8),
    #[error("perfil inválido: {}", .0.join("; "))]
    Invalid(Vec<String>),
    /// Stopped partway: `sent` reached the mouse, `pending` did not.
    #[error("envio interrompido: {message}")]
    Io { sent: Vec<Section>, pending: Vec<Section>, message: String },
}

pub type Opener = Box<dyn Fn() -> Result<Box<dyn Link>, HidError> + Send + Sync>;
pub type OnChange = Box<dyn Fn(&Status) + Send + Sync>;

#[derive(Default)]
struct Seen {
    last_event_ms: Option<u64>,
    model: Option<u8>,
    battery: Option<(BatteryState, u8)>,
    dpi_stage: Option<u8>,
    published: Option<Status>,
}

struct Inner {
    opener: Opener,
    clock: Arc<dyn Clock>,
    on_change: OnChange,
    link: Mutex<Option<Box<dyn Link>>>,
    seen: Mutex<Seen>,
}

#[derive(Clone)]
pub struct DeviceService {
    inner: Arc<Inner>,
}

impl DeviceService {
    /// Build the service without a background thread; call `poll_once` to drive it.
    pub fn new(opener: Opener, clock: Arc<dyn Clock>, on_change: OnChange) -> Self {
        DeviceService {
            inner: Arc::new(Inner {
                opener,
                clock,
                on_change,
                link: Mutex::new(None),
                seen: Mutex::new(Seen::default()),
            }),
        }
    }

    /// Build the service and drive it from a background thread.
    pub fn spawn(opener: Opener, clock: Arc<dyn Clock>, on_change: OnChange) -> Self {
        let svc = Self::new(opener, clock, on_change);
        let worker = svc.clone();
        std::thread::spawn(move || loop {
            if !worker.poll_once() {
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        svc
    }

    /// The real receiver, with a real clock.
    pub fn spawn_default(on_change: OnChange) -> Self {
        Self::spawn(
            Box::new(|| Device::open().map(|d| Box::new(d) as Box<dyn Link>)),
            Arc::new(SystemClock::default()),
            on_change,
        )
    }

    /// One step: (re)open the receiver if needed, read at most one event,
    /// publish any status change. Returns false when there is no receiver.
    pub fn poll_once(&self) -> bool {
        let connected = {
            let mut link = self.inner.link.lock().unwrap();
            if link.is_none() {
                *link = (self.inner.opener)().ok();
            }
            match link.as_ref().map(|l| l.read_event(100)) {
                Some(Ok(Some(ev))) => {
                    self.record(ev);
                    true
                }
                Some(Ok(None)) => true,
                Some(Err(_)) => {
                    *link = None;
                    false
                }
                None => false,
            }
        };
        self.publish();
        connected
    }

    fn record(&self, ev: Event) {
        let mut seen = self.inner.seen.lock().unwrap();
        seen.last_event_ms = Some(self.inner.clock.now_ms());
        seen.model = Some(ev.model);
        match ev.kind {
            EventKind::Battery { state, percent } => seen.battery = Some((state, percent)),
            EventKind::DpiStage(stage) => seen.dpi_stage = Some(stage),
            _ => {}
        }
    }

    fn publish(&self) {
        let status = self.status();
        let changed = {
            let mut seen = self.inner.seen.lock().unwrap();
            let changed = seen.published.as_ref() != Some(&status);
            if changed {
                seen.published = Some(status.clone());
            }
            changed
        };
        if changed {
            (self.inner.on_change)(&status);
        }
    }

    pub fn status(&self) -> Status {
        let has_link = self.inner.link.lock().unwrap().is_some();
        let seen = self.inner.seen.lock().unwrap();
        let connection = if !has_link {
            Connection::NoReceiver
        } else if self.is_awake(&seen) {
            Connection::Awake
        } else {
            Connection::Asleep
        };
        Status {
            connection,
            model: seen.model,
            battery: seen.battery,
            battery_is_stale: seen.battery.is_some() && connection != Connection::Awake,
            dpi_stage: seen.dpi_stage,
        }
    }

    fn is_awake(&self, seen: &Seen) -> bool {
        seen.last_event_ms
            .is_some_and(|t| self.inner.clock.now_ms().saturating_sub(t) <= AWAKE_WINDOW_MS)
    }

    /// Send the given sections of `profile`, in firmware order.
    pub fn apply(&self, profile: &Profile, sections: &[Section]) -> Result<(), ApplyError> {
        profile.validate().map_err(ApplyError::Invalid)?;
        let invalid = |e: delux_proto::Error| ApplyError::Invalid(vec![e.to_string()]);
        let mut packets = Vec::new();
        for s in Section::ALL.into_iter().filter(|s| sections.contains(s)) {
            let inner = match s {
                Section::Sensor => encode_sensor(&profile.sensor).map_err(invalid)?,
                Section::Polling => encode_polling(profile.polling),
                Section::PowerLight => encode_power_light(&profile.power_light).map_err(invalid)?,
                Section::Buttons => encode_buttons(&profile.buttons).map_err(invalid)?,
            };
            packets.push((s, frames(&inner).map_err(invalid)?));
        }
        self.send(packets)
    }

    /// Restore factory settings. The firmware's reset command (0x0C) changes
    /// nothing observable on its own (verified on hardware), so, like the
    /// vendor driver, it is followed by every section of the factory profile.
    pub fn factory_reset(&self) -> Result<(), ApplyError> {
        let reset = frames(&encode_factory_reset()).expect("constant packet");
        self.send_all(vec![reset]).map_err(|(_, e)| e)?;
        self.apply(&Profile::factory("factory"), &Section::ALL)
    }

    fn send(&self, packets: Vec<(Section, Vec<Frame>)>) -> Result<(), ApplyError> {
        let (sections, frames): (Vec<_>, Vec<_>) = packets.into_iter().unzip();
        self.send_all(frames).map_err(|(done, e)| match e {
            // A gate refused before anything was sent: the plain reason is the whole story.
            ApplyError::NoReceiver | ApplyError::Asleep | ApplyError::UnsupportedModel(_)
                if done == 0 =>
            {
                e
            }
            e => ApplyError::Io {
                sent: sections[..done].to_vec(),
                pending: sections[done..].to_vec(),
                message: match e {
                    ApplyError::Io { message, .. } => message,
                    other => other.to_string(),
                },
            },
        })
    }

    /// Send packets one by one, re-checking every gate before each.
    /// On failure returns how many were sent.
    fn send_all(&self, packets: Vec<Vec<Frame>>) -> Result<(), (usize, ApplyError)> {
        let mut link = self.inner.link.lock().unwrap();
        for (done, frames) in packets.iter().enumerate() {
            let fail = |e| (done, e);
            let l = link.as_ref().ok_or(fail(ApplyError::NoReceiver))?;
            // Pick up heartbeats that arrived while we held the link.
            loop {
                match l.read_event(0) {
                    Ok(Some(ev)) => self.record(ev),
                    Ok(None) => break,
                    Err(_) => {
                        *link = None;
                        return Err(fail(ApplyError::NoReceiver));
                    }
                }
            }
            {
                let seen = self.inner.seen.lock().unwrap();
                if !self.is_awake(&seen) {
                    return Err(fail(ApplyError::Asleep));
                }
                match seen.model {
                    Some(MODEL_M900PRO_8K_MS4) => {}
                    Some(other) => return Err(fail(ApplyError::UnsupportedModel(other))),
                    None => return Err(fail(ApplyError::Asleep)),
                }
            }
            if let Err(e) = link.as_ref().expect("checked above").write_frames(frames) {
                *link = None;
                return Err((done, ApplyError::Io {
                    sent: vec![],
                    pending: vec![],
                    message: e.to_string(),
                }));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delux_proto::{
        encode_buttons, encode_polling, encode_power_light, encode_sensor, frames, BatteryState,
        Event, EventKind,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeClock(AtomicU64);
    impl Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    impl FakeClock {
        fn advance(&self, ms: u64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    /// Shared state behind every link the fake opener hands out.
    #[derive(Default)]
    struct World {
        clock: Arc<FakeClock>,
        events: Mutex<VecDeque<Event>>,
        written: Mutex<Vec<Frame>>,
        writes: AtomicUsize,
        /// Fail the Nth write (1-based); 0 = never.
        fail_write_at: AtomicUsize,
        /// Advance the clock by this much after each successful write.
        advance_on_write_ms: AtomicU64,
        /// Keep a heartbeat queued after each write, like an awake mouse.
        heartbeat_after_write: AtomicBool,
        receiver_present: AtomicBool,
    }

    struct FakeLink(Arc<World>);
    impl Link for FakeLink {
        fn read_event(&self, _timeout_ms: i32) -> Result<Option<Event>, HidError> {
            if !self.0.receiver_present.load(Ordering::SeqCst) {
                return Err(HidError::Io("gone".into()));
            }
            Ok(self.0.events.lock().unwrap().pop_front())
        }
        fn write_frames(&self, f: &[Frame]) -> Result<(), HidError> {
            let n = self.0.writes.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.0.fail_write_at.load(Ordering::SeqCst) {
                self.0.receiver_present.store(false, Ordering::SeqCst);
                return Err(HidError::Io("unplugged".into()));
            }
            self.0.written.lock().unwrap().extend_from_slice(f);
            self.0.clock.advance(self.0.advance_on_write_ms.load(Ordering::SeqCst));
            if self.0.heartbeat_after_write.load(Ordering::SeqCst) {
                self.0.events.lock().unwrap().push_back(battery(0x30, 84));
            }
            Ok(())
        }
    }

    fn battery(model: u8, percent: u8) -> Event {
        Event { model, kind: EventKind::Battery { state: BatteryState::Discharging, percent } }
    }

    fn setup() -> (Arc<World>, DeviceService) {
        let world = Arc::new(World::default());
        world.receiver_present.store(true, Ordering::SeqCst);
        world.heartbeat_after_write.store(true, Ordering::SeqCst);
        let w = world.clone();
        let svc = DeviceService::new(
            Box::new(move || {
                if w.receiver_present.load(Ordering::SeqCst) {
                    Ok(Box::new(FakeLink(w.clone())) as Box<dyn Link>)
                } else {
                    Err(HidError::NotFound)
                }
            }),
            world.clock.clone(),
            Box::new(|_| {}),
        );
        (world, svc)
    }

    fn feed(world: &World, svc: &DeviceService, e: Event) {
        world.events.lock().unwrap().push_back(e);
        svc.poll_once();
    }

    #[test]
    fn no_receiver() {
        let (world, svc) = setup();
        world.receiver_present.store(false, Ordering::SeqCst);
        svc.poll_once();
        assert_eq!(svc.status().connection, Connection::NoReceiver);
        assert_eq!(
            svc.apply(&Profile::default_named("t"), &Section::ALL),
            Err(ApplyError::NoReceiver)
        );
    }

    #[test]
    fn battery_goes_stale() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        let s = svc.status();
        assert_eq!(s.connection, Connection::Awake);
        assert_eq!(s.battery, Some((BatteryState::Discharging, 84)));
        assert!(!s.battery_is_stale);
        world.clock.advance(6001);
        let s = svc.status();
        assert_eq!(s.connection, Connection::Asleep);
        assert_eq!(s.battery, Some((BatteryState::Discharging, 84)));
        assert!(s.battery_is_stale);
    }

    #[test]
    fn dpi_stage_is_tracked() {
        let (world, svc) = setup();
        feed(&world, &svc, Event { model: 0x30, kind: EventKind::DpiStage(4) });
        assert_eq!(svc.status().dpi_stage, Some(4));
    }

    #[test]
    fn apply_refused_when_asleep() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        world.clock.advance(6001);
        assert_eq!(svc.apply(&Profile::default_named("t"), &Section::ALL), Err(ApplyError::Asleep));
        assert!(world.written.lock().unwrap().is_empty());
    }

    #[test]
    fn apply_refused_before_any_event() {
        let (world, svc) = setup();
        svc.poll_once();
        assert_eq!(svc.apply(&Profile::default_named("t"), &Section::ALL), Err(ApplyError::Asleep));
        assert!(world.written.lock().unwrap().is_empty());
    }

    #[test]
    fn sleep_mid_apply_keeps_rest_pending() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        world.heartbeat_after_write.store(false, Ordering::SeqCst);
        world.advance_on_write_ms.store(7000, Ordering::SeqCst);
        let p = Profile::default_named("t");
        match svc.apply(&p, &Section::ALL) {
            Err(ApplyError::Io { sent, pending, .. }) => {
                assert_eq!(sent, vec![Section::Sensor]);
                assert_eq!(pending, vec![Section::Polling, Section::PowerLight, Section::Buttons]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(
            *world.written.lock().unwrap(),
            frames(&encode_sensor(&p.sensor).unwrap()).unwrap()
        );
    }

    #[test]
    fn unplug_mid_apply_then_recovers() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        world.fail_write_at.store(2, Ordering::SeqCst);
        match svc.apply(&Profile::default_named("t"), &Section::ALL) {
            Err(ApplyError::Io { sent, pending, .. }) => {
                assert_eq!(sent, vec![Section::Sensor]);
                assert_eq!(pending, vec![Section::Polling, Section::PowerLight, Section::Buttons]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(svc.status().connection, Connection::NoReceiver);
        // receiver comes back
        world.receiver_present.store(true, Ordering::SeqCst);
        world.events.lock().unwrap().clear();
        svc.poll_once(); // reopens
        feed(&world, &svc, battery(0x30, 80));
        assert_eq!(svc.status().connection, Connection::Awake);
        assert_eq!(svc.apply(&Profile::default_named("t"), &[Section::Polling]), Ok(()));
    }

    #[test]
    fn write_failure_on_first_section_reports_all_pending() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        world.fail_write_at.store(1, Ordering::SeqCst);
        match svc.apply(&Profile::default_named("t"), &[Section::Sensor, Section::Polling]) {
            Err(ApplyError::Io { sent, pending, message }) => {
                assert_eq!(sent, vec![]);
                assert_eq!(pending, vec![Section::Sensor, Section::Polling]);
                assert!(message.contains("unplugged"), "{message}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unsupported_model_blocks_write() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x50, 70));
        assert_eq!(svc.status().battery, Some((BatteryState::Discharging, 70)));
        assert_eq!(svc.status().model, Some(0x50));
        assert_eq!(
            svc.apply(&Profile::default_named("t"), &Section::ALL),
            Err(ApplyError::UnsupportedModel(0x50))
        );
        assert_eq!(svc.factory_reset(), Err(ApplyError::UnsupportedModel(0x50)));
        assert!(world.written.lock().unwrap().is_empty());
    }

    #[test]
    fn factory_reset_sends_reset_then_every_default_section() {
        // Hardware finding: cmd 0x0C alone changes nothing; the vendor driver
        // follows it with all defaults, and so do we.
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        assert_eq!(svc.factory_reset(), Ok(()));
        let p = Profile::factory("x");
        let mut expected = frames(&delux_proto::encode_factory_reset()).unwrap();
        expected.extend(frames(&encode_sensor(&p.sensor).unwrap()).unwrap());
        expected.extend(frames(&encode_polling(p.polling)).unwrap());
        expected.extend(frames(&encode_power_light(&p.power_light).unwrap()).unwrap());
        expected.extend(frames(&encode_buttons(&p.buttons).unwrap()).unwrap());
        assert_eq!(*world.written.lock().unwrap(), expected);
    }

    #[test]
    fn invalid_profile_sends_nothing() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        let mut p = Profile::default_named("t");
        p.sensor.stages[0].dpi = 123;
        assert!(matches!(svc.apply(&p, &Section::ALL), Err(ApplyError::Invalid(_))));
        assert!(world.written.lock().unwrap().is_empty());
    }

    #[test]
    fn apply_order_and_bytes() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        let p = Profile::default_named("t");
        // deliberately out of order: the service sends in firmware order
        let asked = [Section::Buttons, Section::Sensor, Section::PowerLight, Section::Polling];
        assert_eq!(svc.apply(&p, &asked), Ok(()));
        let mut expected = Vec::new();
        expected.extend(frames(&encode_sensor(&p.sensor).unwrap()).unwrap());
        expected.extend(frames(&encode_polling(p.polling)).unwrap());
        expected.extend(frames(&encode_power_light(&p.power_light).unwrap()).unwrap());
        expected.extend(frames(&encode_buttons(&p.buttons).unwrap()).unwrap());
        assert_eq!(*world.written.lock().unwrap(), expected);
    }

    #[test]
    fn apply_only_requested_sections() {
        let (world, svc) = setup();
        feed(&world, &svc, battery(0x30, 84));
        let p = Profile::default_named("t");
        assert_eq!(svc.apply(&p, &[Section::Polling]), Ok(()));
        assert_eq!(*world.written.lock().unwrap(), frames(&encode_polling(p.polling)).unwrap());
    }

    #[test]
    fn on_change_fires_on_new_status() {
        let world = Arc::new(World::default());
        world.receiver_present.store(true, Ordering::SeqCst);
        let calls = Arc::new(AtomicUsize::new(0));
        let (w, c) = (world.clone(), calls.clone());
        let svc = DeviceService::new(
            Box::new(move || Ok(Box::new(FakeLink(w.clone())) as Box<dyn Link>)),
            world.clock.clone(),
            Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        feed(&world, &svc, battery(0x30, 84));
        let after_first = calls.load(Ordering::SeqCst);
        assert!(after_first >= 1);
        feed(&world, &svc, battery(0x30, 84)); // same status → no call
        assert_eq!(calls.load(Ordering::SeqCst), after_first);
        feed(&world, &svc, battery(0x30, 83));
        assert_eq!(calls.load(Ordering::SeqCst), after_first + 1);
    }
}
