use crate::actor::Actor;
use async_trait::async_trait;
use dyn_clone::DynClone;
use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct Addr<A>
where
    A: Actor,
{
    id: Uuid,
    sender: mpsc::Sender<A::Msg>,
}

impl<A> Addr<A>
where
    A: Actor,
{
    pub(crate) fn new(sender: mpsc::Sender<A::Msg>) -> Self {
        Self {
            id: Uuid::new_v4(),
            sender,
        }
    }

    /// Send a message to this actor.
    ///
    /// This will block (asynchronously) if the actor's buffer is full
    ///
    /// # Errors
    ///
    /// This will error if the actor is no longer running.
    pub async fn send(&self, msg: impl Into<A::Msg>) -> Result<(), SendError> {
        self.sender.send(msg.into()).await.map_err(|_| SendError)
    }

    pub fn recipient<M>(self) -> Recipient<M>
    where
        M: 'static + Into<A::Msg> + Send,
    {
        self.into()
    }
}

impl<A> Clone for Addr<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            sender: self.sender.clone(),
        }
    }
}

impl<A> Hash for Addr<A>
where
    A: Actor,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(b"addr:");
        self.id.hash(state)
    }
}

impl<A> PartialEq for Addr<A>
where
    A: Actor,
{
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<A> Eq for Addr<A> where A: Actor {}

#[derive(Debug)]
pub struct SendError;

impl Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "actor stopped")
    }
}

impl Error for SendError {}

#[async_trait]
trait RecipientSender<M>: 'static + Send + Send + DynClone {
    async fn send_to_recipient(&self, msg: M) -> Result<(), SendError>;
}

dyn_clone::clone_trait_object!(<M> RecipientSender<M>);

#[async_trait]
impl<S, M> RecipientSender<M> for mpsc::Sender<S>
where
    S: 'static + Send,
    M: 'static + Send + Into<S>,
{
    async fn send_to_recipient(&self, msg: M) -> Result<(), SendError> {
        self.send(msg.into()).await.map_err(|_| SendError)
    }
}

impl<A, M> From<Addr<A>> for Recipient<M>
where
    A: Actor,
    M: 'static + Send + Into<A::Msg>,
{
    fn from(addr: Addr<A>) -> Self {
        Self {
            id: addr.id,
            sender: Box::new(addr.sender),
        }
    }
}

pub struct Recipient<M>
where
    M: 'static,
{
    id: Uuid,
    sender: Box<dyn RecipientSender<M> + Send + Sync>,
}

impl<M> Recipient<M> {
    /// Send a message to the recipient.
    ///
    /// This will block (asynchronously) if the recipient's buffer is full
    ///
    /// # Errors
    ///
    /// This will error if the recipient is no longer running.
    pub async fn send(&self, msg: impl Into<M>) -> Result<(), SendError> {
        self.sender.send_to_recipient(msg.into()).await
    }
}

impl<M> Clone for Recipient<M> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            sender: self.sender.clone(),
        }
    }
}

impl<M> Hash for Recipient<M> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(b"recipient:");
        self.id.hash(state)
    }
}

impl<M> PartialEq for Recipient<M> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<M> Eq for Recipient<M> {}
