use std::{fmt::Display, sync::Mutex};

use anyhow::anyhow;

pub trait Logger: Clone {
    fn log(&self, message: impl Display);
    fn warn(&self, message: impl Display) {
        self.log(format!("WARN: {}", message));
    }
    fn error(&self, message: impl Display) {
        self.log(format!("ERROR: {}", message));
    }
}

#[macro_export]
#[cfg(debug_assertions)]
macro_rules! debug_logf {
    ($logger:expr, $fmt:expr $(, $arg:expr)*) => {
            $logger.log(format!($fmt $(, $arg)*));
    };
}
#[cfg(not(debug_assertions))]
macro_rules! debug_logf {
    ($logger:expr, $fmt:expr $(, $arg:expr)*) => {};
}

impl<T: Logger> Logger for &T {
    fn log(&self, message: impl Display) {
        (*self).log(message);
    }
}

#[derive(Clone)]
pub struct StdioLogger {
    zero_time: std::time::Instant,
}
impl Logger for StdioLogger {
    fn log(&self, message: impl Display) {
        let delta_time = std::time::Instant::now().duration_since(self.zero_time);
        println!("[{:.04}] {}", delta_time.as_secs_f64(), message);
    }
}
impl StdioLogger {
    pub fn new() -> Self {
        Self {
            zero_time: std::time::Instant::now(),
        }
    }
}
impl Default for StdioLogger {
    fn default() -> Self {
        Self::new()
    }
}

pub struct VecLogger {
    logs: Mutex<Vec<String>>,
}

impl Logger for &VecLogger {
    fn log(&self, message: impl Display) {
        self.logs
            .lock()
            .expect("locking the logger array should not fail!")
            .push(message.to_string());
    }
}
impl VecLogger {
    pub fn new() -> Self {
        Self {
            logs: Mutex::new(Vec::new()),
        }
    }

    pub fn get_logs(self) -> Result<Vec<String>, anyhow::Error> {
        // clone the data out of the logger
        self.logs
            .try_lock()
            .map_err(|err| anyhow!("error unlocking VecLogger logs:{err}"))
            .map(|mut x| x.drain(0..).collect::<Vec<_>>())
    }
}
impl Default for VecLogger {
    fn default() -> Self {
        Self::new()
    }
}
