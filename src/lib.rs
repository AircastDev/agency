pub use async_trait::async_trait;
use dyn_clone::DynClone;
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::{collections::VecDeque, future::Future, hash::Hash};
use tokio::{
    select,
    sync::mpsc::{self, channel, unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};
use uuid::Uuid;

pub enum StoppingResult {
    Recover,
    Stop,
}

#[async_trait]
pub trait Actor: Send + Sync + Sized {
    type Msg: 'static + Send + Sync;

    async fn run(&mut self, ctx: &mut Context<Self>);

    async fn init(&mut self, _ctx: &mut Context<Self>) {}

    /// Called after ctx.stop() is called.
    ///
    /// Can be used to restart try and recover the actor and restart the run loop.
    async fn stopping(&mut self, _ctx: &mut Context<Self>) -> StoppingResult {
        StoppingResult::Stop
    }

    async fn stopped(self, _ctx: &mut Context<Self>) {}
}

#[async_trait]
pub trait Setup: Actor {
    type Args: Send + Sync;

    async fn setup(ctx: &mut Context<Self>, args: Self::Args) -> Option<Self>;
}

pub struct AgencyHandle {
    futures: FuturesUnordered<JoinHandle<()>>,
    channel: (
        UnboundedSender<JoinHandle<()>>,
        UnboundedReceiver<JoinHandle<()>>,
    ),
}

impl AgencyHandle {
    fn new() -> Self {
        Self {
            futures: FuturesUnordered::new(),
            channel: unbounded_channel(),
        }
    }

    fn spawner(&self) -> Spawner {
        Spawner::new(self.channel.0.clone())
    }

    pub async fn wait(mut self) {
        loop {
            select! {
                biased;
                fut = self.channel.1.recv() => {
                    self.futures.push(fut.expect("sender is held by the handle"));
                }
                res = self.futures.next() => {
                    // TODO: i think we can catch and log panics here?
                    if res.is_none() {
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Spawner {
    sender: UnboundedSender<JoinHandle<()>>,
}

impl Spawner {
    fn new(sender: UnboundedSender<JoinHandle<()>>) -> Self {
        Self { sender }
    }

    fn spawn<T>(&self, fut: T)
    where
        T: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::task::spawn(fut);
        self.sender
            .send(handle)
            .expect("attempt to spawn task after the handler was dropped");
    }
}

#[derive(Clone)]
pub struct Agency {
    spawner: Spawner,
}

impl Agency {
    pub fn new() -> (Self, AgencyHandle) {
        let handle = AgencyHandle::new();
        (
            Agency {
                spawner: handle.spawner(),
            },
            handle,
        )
    }

    pub fn hire<A>(&self, mut actor: A) -> Addr<A>
    where
        A: 'static + Actor,
    {
        let mut ctx = Context::new(self.clone());
        let addr = ctx.address();
        self.spawner.spawn(async move {
            actor.init(&mut ctx).await;

            loop {
                while !ctx.stopped {
                    actor.run(&mut ctx).await;
                }

                match actor.stopping(&mut ctx).await {
                    StoppingResult::Recover => ctx.stopped = false,
                    StoppingResult::Stop => {
                        break;
                    }
                }
            }

            actor.stopped(&mut ctx).await;
        });
        addr
    }

    pub fn hire_with<A>(&self, args: A::Args) -> Addr<A>
    where
        A: 'static + Setup,
    {
        let mut ctx = Context::new(self.clone());
        let addr = ctx.address();
        self.spawner.spawn(async move {
            if let Some(mut actor) = A::setup(&mut ctx, args).await {
                actor.init(&mut ctx).await;

                loop {
                    while !ctx.stopped {
                        actor.run(&mut ctx).await;
                    }

                    match actor.stopping(&mut ctx).await {
                        StoppingResult::Recover => ctx.stopped = false,
                        StoppingResult::Stop => {
                            break;
                        }
                    }
                }

                actor.stopped(&mut ctx).await;
            }
        });
        addr
    }
}

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
    fn new(sender: mpsc::Sender<A::Msg>) -> Self {
        Self {
            id: Uuid::new_v4(),
            sender,
        }
    }

    pub async fn send(&self, msg: impl Into<A::Msg>) {
        if self.sender.send(msg.into()).await.is_err() {
            // TODO: probably better to bubble this up
            eprintln!("failed to send msg");
        }
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

#[async_trait]
trait RecipientSender<M>: 'static + Send + Send + DynClone {
    async fn send_to_recipient(&self, msg: M);
}

dyn_clone::clone_trait_object!(<M> RecipientSender<M>);

#[async_trait]
impl<S, M> RecipientSender<M> for mpsc::Sender<S>
where
    S: 'static + Send,
    M: 'static + Send + Into<S>,
{
    async fn send_to_recipient(&self, msg: M) {
        if self.send(msg.into()).await.is_err() {
            // TODO: probably better to bubble this up
            eprintln!("failed to send msg");
        }
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
    pub async fn send(&self, msg: impl Into<M>) {
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

pub struct Context<A>
where
    A: Actor,
{
    mailbox: mpsc::Receiver<A::Msg>,
    priority: VecDeque<A::Msg>,
    stopped: bool,
    addr: Addr<A>,
    pub agency: Agency,
}

impl<A> Context<A>
where
    A: Actor,
{
    fn new(agency: Agency) -> Self {
        let (sender, receiver) = channel(16);
        Self {
            mailbox: receiver,
            priority: VecDeque::new(),
            stopped: false,
            addr: Addr::new(sender),
            agency,
        }
    }

    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> A::Msg {
        if let Some(msg) = self.priority.pop_back() {
            return msg;
        }

        self.mailbox
            .recv()
            .await
            .expect("channel cannot be closed whilst context lives")
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    pub fn address(&self) -> Addr<A> {
        self.addr.clone()
    }

    /// Send a message back to this actor.
    ///
    /// Messages sent this way take priority over regular messages.
    pub fn notify(&mut self, msg: impl Into<A::Msg>) {
        self.priority.push_front(msg.into());
    }
}
