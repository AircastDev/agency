use std::{
    error::Error,
    fmt::{Debug, Display},
};
use tokio::sync::oneshot;

pub struct Request<Req, Res> {
    payload: Req,
    reply_to: oneshot::Sender<Res>,
}

impl<Req, Res> Request<Req, Res> {
    pub(crate) fn new(payload: Req) -> (Self, oneshot::Receiver<Res>) {
        let (reply_to, receiver) = oneshot::channel();
        (Self { payload, reply_to }, receiver)
    }

    /// Get the request payload and reponse channel. This returns None if the request sender has
    /// since stopped listening for a response, such as if it reached a timeout.
    pub fn handle(self) -> Option<(Req, oneshot::Sender<Res>)> {
        if self.reply_to.is_closed() {
            None
        } else {
            Some((self.payload, self.reply_to))
        }
    }
}

#[derive(Debug)]
pub enum RequestError {
    ActorStopped,
    SenderDropped,
}

impl Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActorStopped => {
                write!(f, "the actor was stopped before the request could be sent")
            }
            Self::SenderDropped => {
                write!(f, "sender was dropped before responding to the request")
            }
        }
    }
}

impl Error for RequestError {}

#[derive(Debug)]
pub enum RequestTimeoutError {
    ActorStopped,
    SenderDropped,
    Timeout,
}

impl Display for RequestTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActorStopped => {
                write!(f, "the actor was stopped before the request could be sent")
            }
            Self::SenderDropped => {
                write!(f, "sender was dropped before responding to the request")
            }
            Self::Timeout => {
                write!(f, "timeout waiting for response")
            }
        }
    }
}

impl Error for RequestTimeoutError {}
