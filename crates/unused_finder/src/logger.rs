use std::sync::Mutex;

pub trait Logger: Send + Sync + Copy {
    fn log(&self, message: impl Into<String>);
}

pub struct StdioLogger {}
impl Logger for &StdioLogger {
    fn log(&self, message: impl Into<String>) {
        println!("{}", message.into());
    }
}
impl StdioLogger {
    pub fn new() -> Self {
        Self {}
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
    fn log(&self, message: impl Into<String>) {
        self.logs
            .lock()
            .expect("locking the logger array should not fail!")
            .push(message.into());
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
