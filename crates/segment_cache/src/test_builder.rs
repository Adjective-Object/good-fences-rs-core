//! Deliberately unvalidated records for corruption tests, never a production mutation API.
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

pub(super) fn corrupt<T: Serialize + DeserializeOwned>(
    original: &T,
    edit: impl FnOnce(&mut Value),
) -> T {
    let mut value = serde_json::to_value(original).unwrap();
    edit(&mut value);
    serde_json::from_value(value).unwrap()
}

pub(super) trait RecordBuilder {
    fn at(&mut self, path: &str) -> &mut Value;
    fn set(&mut self, path: &str, value: impl Serialize) {
        *self.at(path) = serde_json::to_value(value).unwrap();
    }
    fn copy(&mut self, target: &str, source: &str) {
        let value = self.at(source).clone();
        self.set(target, value);
    }
    fn clear(&mut self, path: &str) {
        match self.at(path) {
            Value::Array(values) => values.clear(),
            Value::String(value) => value.clear(),
            _ => panic!("not a collection: {path}"),
        }
    }
    fn pop(&mut self, path: &str) {
        self.at(path).as_array_mut().unwrap().pop();
    }
    fn push(&mut self, path: &str, value: impl Serialize) {
        self.at(path)
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(value).unwrap());
    }
    fn add(&mut self, path: &str, amount: u64) {
        let value = self.at(path).as_u64().unwrap() + amount;
        self.set(path, value);
    }
    fn reverse(&mut self, path: &str) {
        self.at(path).as_array_mut().unwrap().reverse();
    }
}

impl RecordBuilder for Value {
    fn at(&mut self, path: &str) -> &mut Value {
        let mut current = self;
        for part in path.split('.') {
            current = if current.is_array() {
                let index = match part {
                    "start" => 0,
                    "end" => 1,
                    _ => part.parse().unwrap(),
                };
                &mut current[index]
            } else {
                current
                    .get_mut(part)
                    .unwrap_or_else(|| panic!("missing field {part}"))
            };
        }
        current
    }
}
