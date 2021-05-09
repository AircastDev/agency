use async_executors::{TokioTp, TokioTpBuilder};
use async_nursery::{NurseExt, Nursery};
pub use async_trait::async_trait;
use std::{collections::VecDeque, future::Future};
use tokio::sync::mpsc;

#[async_trait]
pub trait Actor: Send + Sync + Sized {
    type Msg: Send + Sync;

    async fn run(&mut self, ctx: &mut Context<Self>);

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {}
}

#[async_trait]
pub trait Setup: Actor {
    type Args: Send + Sync;

    async fn setup(ctx: &mut Context<Self>, args: Self::Args) -> Option<Self>;
}

#[derive(Clone)]
pub struct Agency {
    nursery: Nursery<TokioTp, ()>,
}

impl Agency {
    pub fn new() -> (Self, impl Future<Output = ()>) {
        let mut exec_builder = TokioTpBuilder::new();
        exec_builder.tokio_builder().enable_all();
        let exec = exec_builder.build().expect("create tokio threadpool");

        let (nursery, nursery_stream) = Nursery::new(exec);
        (Agency { nursery }, nursery_stream)
    }

    pub fn hire<A>(&self, mut actor: A) -> Addr<A>
    where
        A: 'static + Actor,
    {
        let (sender, receiver) = mpsc::channel(16);
        let addr = Addr::new(sender);
        let mut ctx = Context {
            mailbox: receiver,
            priority: VecDeque::new(),
            stopped: false,
            addr: addr.clone(),
            agency: self.clone(),
        };
        self.nursery
            .nurse(async move {
                while !ctx.stopped {
                    actor.run(&mut ctx).await;
                }

                actor.stopped(&mut ctx).await;
            })
            .unwrap();
        addr
    }

    pub fn hire_with<A>(&self, args: A::Args) -> Addr<A>
    where
        A: 'static + Setup,
    {
        let (sender, receiver) = mpsc::channel(16);
        let addr = Addr::new(sender);
        let mut ctx = Context {
            mailbox: receiver,
            priority: VecDeque::new(),
            stopped: false,
            addr: addr.clone(),
            agency: self.clone(),
        };
        self.nursery
            .nurse(async move {
                if let Some(mut actor) = A::setup(&mut ctx, args).await {
                    while !ctx.stopped {
                        actor.run(&mut ctx).await;
                    }

                    actor.stopped(&mut ctx).await;
                }
            })
            .unwrap();
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
    fn new(sender: mpsc::Sender<A::Msg>) -> Self {
        Self {
            recipient: Recipient::new(sender),
        }
    }

    pub async fn send(&self, msg: A::Msg) {
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
    sender: mpsc::Sender<T>,
}

impl<T> Recipient<T> {
    fn new(sender: mpsc::Sender<T>) -> Self {
        Self { sender }
    }

    pub async fn send(&self, msg: T) {
        if self.sender.send(msg).await.is_err() {
            eprint!("failed to send msg");
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
    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> Option<A::Msg> {
        if let Some(msg) = self.priority.pop_back() {
            return Some(msg);
        }

        self.mailbox.recv().await
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
    pub fn notify(&mut self, msg: A::Msg) {
        self.priority.push_front(msg);
    }
}
