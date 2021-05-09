//
// - Each actor has its own 'thread'
// - Actor has a run method that is called in a loop
// - Actor can select over incoming messages and any other async stream-like thing

use async_executors::{TokioTp, TokioTpBuilder};
use async_nursery::{NurseExt, Nursery, NurseryStream};
pub use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait Actor: Send + Sync + Sized {
    type Msg: Send + Sync;

    async fn run(&mut self, mailbox: &mut Context<Self>);
}

pub struct Agency {
    nursery: Nursery<TokioTp, ()>,
    nursery_stream: NurseryStream<()>,
}

impl Agency {
    pub fn new() -> Self {
        let mut exec_builder = TokioTpBuilder::new();
        exec_builder.tokio_builder().enable_all();
        let exec = exec_builder.build().expect("create tokio threadpool");

        let (nursery, nursery_stream) = Nursery::new(exec);
        Agency {
            nursery,
            nursery_stream,
        }
    }

    pub fn hire<A>(&self, mut actor: A) -> Addr<A>
    where
        A: 'static + Actor,
    {
        let (sender, receiver) = mpsc::channel(16);
        let addr = Addr::new(sender);
        let mut ctx = Context {
            mailbox: receiver,
            stopped: false,
            addr: addr.clone(),
        };
        self.nursery
            .nurse(async move {
                while !ctx.stopped {
                    actor.run(&mut ctx).await;
                }
            })
            .unwrap();
        addr
    }

    pub async fn wait(self) {
        self.nursery_stream.await;
    }
}

impl Default for Agency {
    fn default() -> Self {
        Self::new()
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
    stopped: bool,
    addr: Addr<A>,
}

impl<A> Context<A>
where
    A: Actor,
{
    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> Option<A::Msg> {
        self.mailbox.recv().await
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    pub fn address(&self) -> Addr<A> {
        self.addr.clone()
    }
}
