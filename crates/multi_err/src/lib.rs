use std::fmt::Display;

pub struct MultiErr<TErr> {
    errs: Vec<TErr>,
}

impl<TErr> MultiErr<TErr> {
    pub fn new() -> Self {
        Self { errs: Vec::new() }
    }

    pub fn add_multi(&mut self, other: MultiErr<TErr>) {
        self.errs.extend(other.errs);
    }

    pub fn add_iter(&mut self, other: impl Iterator<Item = TErr>) {
        self.errs.extend(other);
    }

    pub fn add_single(&mut self, other: TErr) {
        self.errs.push(other);
    }

    // Convenience wrapper for add_multi for unpacking a result tuple
    pub fn extract<T>(&mut self, other: MultiResult<T, TErr>) -> T {
        self.add_multi(other.1);
        other.0
    }

    // Convenience wrapper for add_multi for unpacking a result tuple
    pub fn extract_multi<T>(&mut self, other: (T, MultiErr<TErr>)) -> T {
        self.add_multi(other.1);
        other.0
    }

    // Convenience wrapper for add_iter for unpacking a result tuple
    pub fn extract_iter<T>(&mut self, other: (T, impl Iterator<Item = TErr>)) -> T {
        self.add_iter(other.1);
        other.0
    }

    // Convenience wrapper for add_single for unpacking a result tuple
    pub fn extract_single<T>(&mut self, other: (T, TErr)) -> T {
        self.add_single(other.1);
        other.0
    }

    pub fn with_value<T>(self, val: T) -> MultiResult<T, TErr> {
        MultiResult::with_errs(val, self)
    }

    // converts this mutli error into a result
    pub fn into_result(self) -> Result<(), Self> {
        if self.errs.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}
impl<TErr> Default for MultiErr<TErr> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> From<MultiErr<T>> for Vec<T> {
    fn from(other: MultiErr<T>) -> Self {
        other.errs
    }
}
impl<T: Display> MultiErr<T> {
    fn into_anyhow(self) -> anyhow::Error {
        if self.errs.len() == 1 {
            return anyhow::anyhow!("{:#}", self.errs[0]);
        }

        anyhow::anyhow!(
            "{} errors:\n  {}",
            self.errs.len(),
            self.errs
                .iter()
                .enumerate()
                .map(|(i, e)| format!("{}: {:#}", i + 1, e))
                .collect::<Vec<String>>()
                .join("\n  ")
        )
    }
}
impl<T: Display> From<MultiErr<T>> for anyhow::Error {
    fn from(other: MultiErr<T>) -> Self {
        other.into_anyhow()
    }
}

pub struct MultiResult<TRes, TErr>(TRes, MultiErr<TErr>);
impl<TRes, TErr> MultiResult<TRes, TErr> {
    pub fn from(val: TRes) -> Self {
        Self(val, MultiErr::new())
    }
    pub fn with_errs(val: TRes, errs: MultiErr<TErr>) -> Self {
        Self(val, errs)
    }
}

impl<TRes, TErr: Display> MultiResult<TRes, TErr> {
    // converts this mutli error into a result
    pub fn into_anyhow(self) -> Result<TRes, anyhow::Error> {
        if self.1.errs.is_empty() {
            Ok(self.0)
        } else {
            Err(self.1.into_anyhow())
        }
    }
}

impl<TRes, TErr> From<MultiResult<TRes, TErr>> for Result<TRes, MultiErr<TErr>> {
    fn from(multi_result: MultiResult<TRes, TErr>) -> Self {
        let (val, multi_errs) = (multi_result.0, multi_result.1);
        multi_errs.into_result().map(|_| val)
    }
}

#[cfg(test)]
mod test {
    use anyhow::anyhow;
    use pretty_assertions::assert_eq;

    use crate::MultiErr;

    #[test]
    fn test_into_anyhow_display_single() {
        let mut multi_err = MultiErr::new();
        let e = anyhow!("this is the error").context("this is the context");
        multi_err.add_single(e);

        let as_str = format!("{:#}", multi_err.into_anyhow());
        assert_eq!(as_str, "this is the context: this is the error");
    }

    #[test]
    fn test_into_anyhow_display_multi() {
        let mut multi_err = MultiErr::new();
        multi_err.add_single(anyhow!("this is the first error").context("this is the context"));
        multi_err.add_single(anyhow!("this is the second error"));

        let as_str = format!("{:#}", multi_err.into_anyhow());
        assert_eq!(
            as_str,
            "2 errors:
  1: this is the context: this is the first error
  2: this is the second error"
        )
    }

    #[test]
    fn test_into_anyhow_display_multi_conext_chain() {
        let mut multi_err = MultiErr::new();
        multi_err.add_single(anyhow!("this is the first error").context("this is the context"));
        multi_err.add_single(anyhow!("this is the second error"));

        let as_str = format!(
            "{:#}",
            multi_err.into_anyhow().context("this is the outer context")
        );
        assert_eq!(
            as_str,
            "this is the outer context: 2 errors:
  1: this is the context: this is the first error
  2: this is the second error"
        )
    }
}
