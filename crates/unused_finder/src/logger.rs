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
