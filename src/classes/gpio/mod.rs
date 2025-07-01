use crate::SYSFS_DIR;
use libc::{CLOCK_MONOTONIC, PR_SET_TIMERSLACK, SCHED_RR, sched_param, timespec};
use log::error;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::{File, create_dir_all};
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread::sleep;
use std::time::Duration;
use tokio::task::JoinHandle;

pub const GPIO_CLASS: &str = "gpio";

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[repr(u8)]
pub enum PinLevel {
    Low = 0,
    High = 1,
}
impl AsRef<[u8]> for PinLevel {
    fn as_ref(&self) -> &[u8] {
        match self {
            PinLevel::Low => b"0",
            PinLevel::High => b"1",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum PinPull {
    Off = 0b00,
    Down = 0b01,
    Up = 0b10,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum PinMode {
    Output,
    Input,
}
impl AsRef<[u8]> for PinMode {
    fn as_ref(&self) -> &[u8] {
        match self {
            PinMode::Output => b"out",
            PinMode::Input => b"in",
        }
    }
}

pub struct Pin {
    number: u8,
    value: File,
    mode: File,
}
impl Pin {
    pub fn new(number: u8, pin_mode: PinMode) -> Result<Self, Error> {
        let (value, mode) = Self::export(number)
            .map_err(|e| Error::new(e.kind(), format!("Error Exporting Pin: {:?}", e)))?;
        let mut slf = Self {
            number,
            value,
            mode,
        };
        slf.mode(pin_mode)
            .map_err(|e| Error::new(e.kind(), format!("Error Setting Pin Mode: {:?}", e)))?;
        Ok(slf)
    }
    pub fn read_value(&mut self) -> Result<u8, Error> {
        let mut s = String::with_capacity(10);
        self.value.seek(SeekFrom::Start(0))?;
        self.value.read_to_string(&mut s)?;
        s[..1].parse::<u8>().map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to parse value of pin {}: {:?}", self.number, e),
            )
        })
    }
    fn set(&mut self, value: PinLevel) -> Result<(), Error> {
        self.value.write_all(value.as_ref())
    }
    fn mode(&mut self, mode: PinMode) -> Result<(), Error> {
        self.mode.write_all(mode.as_ref())
    }
    fn export(number: u8) -> Result<(File, File), Error> {
        let pin_str = format!("{SYSFS_DIR}/{GPIO_CLASS}/gpio{number}/");
        let pin_path = Path::new(&pin_str);
        if !pin_path.exists() {
            let mut export_file = File::create(format!("{SYSFS_DIR}/{GPIO_CLASS}/export"))?;
            export_file.write_all(&[number])?;
        }
        create_dir_all(format!("{SYSFS_DIR}/{GPIO_CLASS}/gpio{number}/"))?;
        Ok((
            File::create(format!("{SYSFS_DIR}/{GPIO_CLASS}/gpio{number}/value"))?,
            File::create(format!("{SYSFS_DIR}/{GPIO_CLASS}/gpio{number}/direction"))?,
        ))
    }
    fn unexport(number: u8) -> Result<(), Error> {
        let mut unexport_file = File::create(format!("{SYSFS_DIR}/{GPIO_CLASS}/unexport"))?;
        unexport_file.write_all(&[number])?;
        Ok(())
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        Self::unexport(self.number).unwrap_or_default()
    }
}
pub struct PinSet {
    pins: HashMap<u8, PwmSignalHandler>,
}
impl PinSet {
    pub async fn get_or_init(&mut self, pin: u8) -> Result<&mut PwmSignalHandler, Error> {
        if let Entry::Vacant(e) = self.pins.entry(pin) {
            e.insert(PwmSignalHandler::new(
                Pin::new(pin, PinMode::Output)?,
                Duration::from_nanos(0),
                Duration::from_nanos(0),
            ));
        }
        Ok(self
            .pins
            .get_mut(&pin)
            .expect("Occupied Entry was None or failed to insert, Should not happen"))
    }

    pub async fn stop_all(self) -> Result<(), Error> {
        for mut pin in self.pins.into_values() {
            pin.stop();
            match pin.signal_thread.await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => error!("Error stopping pin thread: {e:?}"),
                Err(e) => error!("Error joining pin thread: {e:?}"),
            }
        }
        Ok(())
    }
    pub fn take(&mut self, pin: u8) -> Option<PwmSignalHandler> {
        self.pins.remove(&pin)
    }
    pub fn set_handler(&mut self, pin: u8, handler: PwmSignalHandler) -> Option<PwmSignalHandler> {
        self.pins.insert(pin, handler)
    }
    pub fn pins(&mut self) -> &mut HashMap<u8, PwmSignalHandler> {
        &mut self.pins
    }
}

pub const DEFAULT_PWN_PERIOD_US: u64 = 20000;

pub struct PwmSignalSettings {
    period: Duration,
    pulse_width: Duration,
}

pub enum PwmSignal {
    Stop,
    Update(PwmSignalSettings),
}

pub struct PwmSignalHandler {
    signal_thread: JoinHandle<Result<(), Error>>,
    control_channel: Sender<PwmSignal>,
}

const SLEEP_THRESHOLD: i64 = 250_000;
const BUSYWAIT_MAX: i64 = 200_000;
const BUSYWAIT_REMAINDER: i64 = 100;

impl PwmSignalHandler {
    fn new(mut pin: Pin, period: Duration, pulse_width: Duration) -> PwmSignalHandler {
        let (sender, receiver) = std::sync::mpsc::channel();
        PwmSignalHandler {
            signal_thread: tokio::task::spawn_blocking(move || {
                // Set the scheduling policy to real-time round robin at the highest priority. This
                // will silently fail if we're not running as root.
                unsafe {
                    let mut params = MaybeUninit::<sched_param>::zeroed().assume_init();
                    params.sched_priority = libc::sched_get_priority_max(SCHED_RR);
                    libc::sched_setscheduler(0, SCHED_RR, &params);
                    // Set timer slack to 1 ns (default = 50 µs). This is only relevant if we're unable
                    // to set a real-time scheduling policy.
                    libc::prctl(PR_SET_TIMERSLACK, 1);
                }
                let mut period_ns = period.as_nanos() as i64;
                let mut pulse_width_ns = pulse_width.as_nanos() as i64;
                let mut start_ns = get_time_ns();
                loop {
                    //Receive Updates
                    while let Ok(msg) = receiver.try_recv() {
                        match msg {
                            PwmSignal::Update(settings) => {
                                // Reconfigure period and pulse width
                                pulse_width_ns = settings.pulse_width.as_nanos() as i64;
                                period_ns = settings.period.as_nanos() as i64;
                                if pulse_width_ns > period_ns {
                                    pulse_width_ns = period_ns;
                                }
                            }
                            PwmSignal::Stop => {
                                return Ok(());
                            }
                        }
                    }
                    if pulse_width_ns > 0 {
                        pin.set(PinLevel::High)?;
                        if pulse_width_ns == period_ns {
                            sleep(Duration::from_millis(10));
                            continue;
                        }
                    } else {
                        pin.set(PinLevel::Low)?;
                        sleep(Duration::from_millis(10));
                        continue;
                    }
                    // Sleep if we have enough time remaining, while reserving some time
                    // for busy waiting to compensate for sleep taking longer than needed.
                    if pulse_width_ns >= SLEEP_THRESHOLD {
                        sleep(Duration::from_nanos(
                            pulse_width_ns.saturating_sub(BUSYWAIT_MAX) as u64,
                        ));
                    }
                    // Busy-wait for the remaining active time, minus BUSYWAIT_REMAINDER
                    // to account for get_time_ns() overhead
                    loop {
                        if pulse_width_ns.saturating_sub(get_time_ns().saturating_sub(start_ns))
                            <= BUSYWAIT_REMAINDER
                        {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                    pin.set(PinLevel::Low)?;
                    let remaining_ns =
                        period_ns.saturating_sub(get_time_ns().saturating_sub(start_ns));
                    // Sleep if we have enough time remaining, while reserving some time
                    // for busy waiting to compensate for sleep taking longer than needed.
                    if remaining_ns >= SLEEP_THRESHOLD {
                        sleep(Duration::from_nanos(
                            remaining_ns.saturating_sub(BUSYWAIT_MAX) as u64,
                        ));
                    }
                    // Busy-wait for the remaining inactive time, minus BUSYWAIT_REMAINDER
                    // to account for get_time_ns() overhead
                    loop {
                        let current_ns = get_time_ns();
                        if period_ns.saturating_sub(current_ns.saturating_sub(start_ns))
                            <= BUSYWAIT_REMAINDER
                        {
                            start_ns = current_ns;
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
            }),
            control_channel: sender,
        }
    }

    pub fn set_pwm(&mut self, period: Duration, pulse_width: Duration) {
        if let Err(e) = self
            .control_channel
            .send(PwmSignal::Update(PwmSignalSettings {
                period,
                pulse_width,
            }))
        {
            error!("Error Setting Pin PWM: {:?}", e);
        }
    }

    pub fn set_high(&mut self) {
        if let Err(e) = self
            .control_channel
            .send(PwmSignal::Update(PwmSignalSettings {
                period: Duration::from_micros(DEFAULT_PWN_PERIOD_US),
                pulse_width: Duration::from_micros(DEFAULT_PWN_PERIOD_US),
            }))
        {
            error!("Error Setting Pin High: {:?}", e);
        }
    }

    pub fn set_low(&mut self) {
        if let Err(e) = self
            .control_channel
            .send(PwmSignal::Update(PwmSignalSettings {
                period: Duration::from_micros(DEFAULT_PWN_PERIOD_US),
                pulse_width: Duration::from_micros(0),
            }))
        {
            error!("Error Setting Pin Low: {:?}", e);
        }
    }

    pub fn stop(&mut self) {
        if let Err(e) = self.control_channel.send(PwmSignal::Stop) {
            error!("Error Stopping Pin: {:?}", e);
        }
    }
}

const NANOS_PER_SEC: i64 = 1_000_000_000;
//Standard Fast Clock for NS
#[inline(always)]
fn get_time_ns() -> i64 {
    let mut ts = timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(CLOCK_MONOTONIC, &mut ts);
    }
    (ts.tv_sec * NANOS_PER_SEC) + ts.tv_nsec
}
