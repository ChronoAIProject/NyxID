use std::fmt;

#[derive(Clone, Copy)]
pub struct RedactedLen(pub usize);

impl fmt::Debug for RedactedLen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted len={}>", self.0)
    }
}
