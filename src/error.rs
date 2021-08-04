use x11rb::errors::{ConnectionError, ReplyError};

pub(crate) type CWMRes<T> = Result<T, CWMError>;

#[derive(Debug)]
pub enum CWMError {
    Conn(ConnectionError),
    Reply(ReplyError)
}

impl From<ConnectionError> for CWMError {
    fn from(other: ConnectionError) -> Self {
        Self::Conn(other)
    }
}

impl From<ReplyError> for CWMError {
    fn from(other: ReplyError) -> Self {
        Self::Reply(other)
    }
}