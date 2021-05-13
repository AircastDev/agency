pub use async_trait::async_trait;
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::{collections::VecDeque, future::Future};
use tokio::{
    select,
    sync::mpsc::{
        channel, unbounded_channel, Receiver, Sender, UnboundedReceiver, UnboundedSender,
    },
    task::JoinHandle,
};

#[async_trait]
pub trait Actor: Send + Sync + Sized {
    type Msg: Send + Sync;

    async fn run(&mut self, ctx: &mut Context<Self>);

    async fn init(&mut self, _ctx: &mut Context<Self>) {}

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

            while !ctx.stopped {
                actor.run(&mut ctx).await;
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

                while !ctx.stopped {
                    actor.run(&mut ctx).await;
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
    recipient: Recipient<A::Msg>,
}

impl<A> Addr<A>
where
    A: Actor,
{
    fn new(sender: Sender<A::Msg>) -> Self {
        Self {
            recipient: Recipient::new(sender),
        }
    }

    pub async fn send(&self, msg: impl Into<A::Msg>) {
        self.recipient.send(msg).await
    }

    pub fn recipient(self) -> Recipient<A::Msg> {
        self.recipient
    }
}

impl<A> Clone for Addr<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            recipient: self.recipient.clone(),
        }
    }
}

pub struct Recipient<T> {
    sender: Sender<T>,
}

impl<T> Recipient<T> {
    fn new(sender: Sender<T>) -> Self {
        Self { sender }
    }

    pub async fn send(&self, msg: impl Into<T>) {
        if self.sender.send(msg.into()).await.is_err() {
            // TODO: probably better to bubble this up
            eprintln!("failed to send msg");
        }
    }
}

impl<T> Clone for Recipient<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

pub struct Context<A>
where
    A: Actor,
{
    mailbox: Receiver<A::Msg>,
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
