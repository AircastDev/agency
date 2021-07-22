use crate::{
    actor::Actor,
    request::{Request, RequestError, RequestTimeoutError},
};
use async_trait::async_trait;
use dyn_clone::DynClone;
use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};
use uuid::Uuid;

pub struct Addr<A>
where
    A: Actor,
{
    id: Uuid,
    mailer: mpsc::Sender<A::Msg>,
    priority_mailer: mpsc::UnboundedSender<A::Msg>,
}

impl<A> Addr<A>
where
    A: Actor,
{
    pub(crate) fn new(
        mailer: mpsc::Sender<A::Msg>,
        priority_mailer: mpsc::UnboundedSender<A::Msg>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            mailer,
            priority_mailer,
        }
    }

    /// Send a message to this actor.
    ///
    /// This will block (asynchronously) if the actor's mailbox is full
    ///
    /// # Errors
    ///
    /// This will error if the actor is no longer running.
    pub async fn send(&self, msg: impl Into<A::Msg>) -> Result<(), SendError> {
        self.mailer.send(msg.into()).await.map_err(|_| SendError)
    }

    /// Send a message to this actor, with a higher priority over regular messages.
    ///
    /// Unlike [`Addr::send`], this will not block as the priority mailbox has infinite capacity. As
    /// a result you should use this sparingly due to the lack of backpressure.
    ///
    /// # Errors
    ///
    /// This will error if the actor is no longer running.
    pub fn send_priority(&self, msg: impl Into<A::Msg>) -> Result<(), SendError> {
        self.priority_mailer.send(msg.into()).map_err(|_| SendError)
    }

    pub fn recipient<M>(self) -> Recipient<M>
    where
        M: 'static + Into<A::Msg> + Send,
    {
        self.into()
    }

    /// Send a [`Request`](crate::Request) to this actor and await the response.
    ///
    /// This could wait indefinitely if the actor never responds, however it will error if the actor
    /// is stopped before or during the request, or if the response sender is otherwise dropped.
    pub async fn request<Req, Res>(&self, payload: Req) -> Result<Res, RequestError>
    where
        Request<Req, Res>: Into<A::Msg>,
    {
        let (request, receiver) = Request::new(payload);
        self.mailer
            .send(request.into())
            .await
            .map_err(|_| RequestError::ActorStopped)?;
        let res = receiver.await.map_err(|_| RequestError::SenderDropped)?;
        Ok(res)
    }

    /// Send a [`Request`](crate::Request) to this actor and await the response.
    ///
    /// This will error if the timeout is reached, if the actor is stopped before or during the
    /// request, or if the response sender is otherwise dropped.
    pub async fn request_timeout<Req, Res>(
        &self,
        payload: Req,
        duration: Duration,
    ) -> Result<Res, RequestTimeoutError>
    where
        Request<Req, Res>: Into<A::Msg>,
    {
        let (request, receiver) = Request::new(payload);
        self.mailer
            .send(request.into())
            .await
            .map_err(|_| RequestTimeoutError::ActorStopped)?;
        let res = timeout(duration, receiver)
            .await
            .map_err(|_| RequestTimeoutError::Timeout)?
            .map_err(|_| RequestTimeoutError::SenderDropped)?;
        Ok(res)
    }
}

impl<A> Clone for Addr<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            mailer: self.mailer.clone(),
            priority_mailer: self.priority_mailer.clone(),
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
            sender: Box::new(addr.mailer),
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

impl<Req, Res> Recipient<Request<Req, Res>> {
    /// Send a [`Request`](crate::Request) to the actor and await the response.
    ///
    /// This could wait indefinitely if the actor never responds, however it will error if the actor
    /// is stopped before or during the request, or if the response sender is otherwise dropped.
    pub async fn request(&self, payload: Req) -> Result<Res, RequestError> {
        let (request, receiver) = Request::new(payload);
        self.sender
            .send_to_recipient(request)
            .await
            .map_err(|_| RequestError::ActorStopped)?;
        let res = receiver.await.map_err(|_| RequestError::SenderDropped)?;
        Ok(res)
    }

    /// Send a [`Request`](crate::Request) to the actor and await the response.
    ///
    /// This will error if the timeout is reached, if the actor is stopped before or during the
    /// request, or if the response sender is otherwise dropped.
    pub async fn request_timeout(
        &self,
        payload: Req,
        duration: Duration,
    ) -> Result<Res, RequestTimeoutError> {
        let (request, receiver) = Request::new(payload);
        self.sender
            .send_to_recipient(request)
            .await
            .map_err(|_| RequestTimeoutError::ActorStopped)?;
        let res = timeout(duration, receiver)
            .await
            .map_err(|_| RequestTimeoutError::Timeout)?
            .map_err(|_| RequestTimeoutError::SenderDropped)?;
        Ok(res)
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
